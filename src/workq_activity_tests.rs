use super::setup;
use crate::{Screen, kernel_model::WorkQueueKey, model, workq_activity};
use gpui::{Modifiers, TestAppContext, VisualTestContext, point, px, size};

fn click(cx: &mut VisualTestContext, id: &'static str) {
    let position = cx.debug_bounds(id).expect(id).center();
    cx.simulate_click(position, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

#[gpui_kit::test]
fn work_queue_activity_uses_shared_zoom_and_keeps_backlog_and_session_independent(
    cx: &mut TestAppContext,
) {
    let (app, mut cx) = setup(cx);
    let key = WorkQueueKey {
        address: 28672,
        name: "system_workq".into(),
    };
    cx.simulate_resize(size(px(1000.), px(680.)));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.frames = model::parse(include_str!("../test-data/workqs.json")).unwrap();
            app.frames[1].workqs[0].flags = Some(1);
            app.frame = 2;
            app.screen = Screen::WorkQueue(key.clone());
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    assert!(cx.debug_bounds("workq-state-Processing").is_some());
    assert!(
        cx.debug_bounds("workq-processing-item").is_some(),
        "processing is reported even though backlog is truncated"
    );
    assert!(cx.debug_bounds("workq-pending-tag-0").is_some());
    assert!(
        cx.debug_bounds("workq-activity-view-0.000-10.000")
            .is_some()
    );
    app.read_with(&cx, |app, _| {
        assert_eq!(
            workq_activity::observed_busy(&app.frames, app.frame, &key).percent(),
            Some(40.)
        )
    });
    let footer = cx.debug_bounds("session-footer").unwrap();
    let activity = cx.debug_bounds("workq-activity").unwrap();
    let summary = cx.debug_bounds("workq-summary").unwrap();
    let history = cx.debug_bounds("workq-history").unwrap();
    assert!(activity.top() >= summary.bottom() && history.top() >= activity.bottom());
    assert!(history.bottom() <= footer.top());
    assert!(
        cx.debug_bounds("workq-backlog-zoom-surface")
            .unwrap()
            .size
            .height
            >= px(80.)
    );
    click(&mut cx, "workq-activity-zoom-in");
    assert!(cx.debug_bounds("workq-activity-view-2.500-7.500").is_some());
    assert!(cx.debug_bounds("workq-backlog-unzoomed").is_some());
    let strip = cx.debug_bounds("workq-activity-strip").unwrap();
    cx.simulate_event(gpui::MouseMoveEvent {
        position: strip.center(),
        ..Default::default()
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("workq-activity-tooltip").is_some());
    click(&mut cx, "workq-activity-strip");
    assert!(cx.debug_bounds("workq-activity-selected-2").is_some());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.frame = 3;
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    assert!(
        cx.debug_bounds("workq-activity-selected-2").is_some(),
        "past inspection stays stable as playback advances"
    );
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.frame = 2;
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    cx.simulate_keystrokes("right");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("workq-activity-view-3.000-8.000").is_some());
    cx.simulate_keystrokes("home");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("workq-activity-view-0.000-5.000").is_some());
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: strip.center(),
        delta: gpui::ScrollDelta::Pixels(point(px(-100.), px(0.))),
        modifiers: Modifiers::default(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("workq-activity-view-0.000-5.000").is_none());
    assert_eq!(cx.debug_bounds("session-footer").unwrap(), footer);
    click(&mut cx, "workq-activity-fit");
    assert!(
        cx.debug_bounds("workq-activity-view-0.000-10.000")
            .is_some()
    );
    click(&mut cx, "workq-activity-zoom-in");
    cx.simulate_resize(size(px(1480.), px(940.)));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("workq-activity-view-2.500-7.500").is_some());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.screen = Screen::WorkQueue(WorkQueueKey {
                address: 28673,
                name: key.name.clone(),
            });
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    assert!(
        cx.debug_bounds("workq-activity-view-0.000-10.000")
            .is_some()
    );
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.screen = Screen::WorkQueue(key.clone());
            app.frame = 1;
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    assert!(cx.debug_bounds("workq-activity-view-2.500-7.500").is_some());
    assert!(cx.debug_bounds("workq-activity-unselected").is_some());
    assert!(cx.debug_bounds("workq-state-Idle").is_some());
    assert!(cx.debug_bounds("workq-processing-item").is_none());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.load(
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-data/workqs.json"),
                window,
                cx,
            );
            app.screen = Screen::WorkQueue(key);
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    assert!(
        cx.debug_bounds("workq-activity-view-0.000-10.000")
            .is_some()
    );
    app.read_with(&cx, |app, _| {
        assert_eq!(app.frame, 0);
        assert!(!app.playing);
    });
}

#[gpui_kit::test]
fn work_queue_status_handles_unknown_ready_idle_stopped_and_single_frames(cx: &mut TestAppContext) {
    let (app, mut cx) = setup(cx);
    let key = WorkQueueKey {
        address: 28672,
        name: "system_workq".into(),
    };
    for (flags, pending, truncated, state, processing, pressure) in [
        (
            Some(3),
            Some(vec![]),
            false,
            "workq-state-Processing",
            true,
            "active",
        ),
        (
            Some(3),
            Some(vec!["task".to_string()]),
            true,
            "workq-state-Processing",
            true,
            "backlog",
        ),
        (
            Some(3),
            None,
            false,
            "workq-state-Processing",
            true,
            "unknown",
        ),
        (
            Some(3),
            Some(vec![]),
            true,
            "workq-state-Processing",
            true,
            "unknown",
        ),
        (
            Some(1),
            Some(vec!["task".to_string()]),
            false,
            "workq-state-Ready",
            false,
            "backlog",
        ),
        (
            None,
            Some(vec!["task".to_string()]),
            false,
            "workq-state-Unknown",
            false,
            "backlog",
        ),
        (
            Some(1),
            Some(vec![]),
            false,
            "workq-state-Idle",
            false,
            "clear",
        ),
        (
            None,
            Some(vec![]),
            false,
            "workq-state-Unknown",
            false,
            "unknown",
        ),
        (
            Some(0),
            Some(vec![]),
            false,
            "workq-state-Not started",
            false,
            "clear",
        ),
        (
            Some(1),
            None,
            false,
            "workq-state-Not processing",
            false,
            "unknown",
        ),
        (
            Some(1),
            Some(vec![]),
            true,
            "workq-state-Not processing",
            false,
            "unknown",
        ),
    ] {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.frames = model::parse(include_str!("../test-data/workqs.json")).unwrap();
                app.frames.truncate(1);
                app.frames[0].elapsed_seconds = None;
                let queue = &mut app.frames[0].workqs[0];
                queue.flags = flags;
                queue.pending = pending.clone();
                queue.pending_truncated = truncated;
                app.frame = 0;
                app.screen = Screen::WorkQueue(key.clone());
                cx.notify();
            });
            window.draw(cx).clear(cx);
        });
        assert!(cx.debug_bounds(state).is_some(), "{state}");
        let selector = match pressure {
            "active" => "workq-pressure-active",
            "backlog" => "workq-pressure-backlog",
            "clear" => "workq-pressure-clear",
            "unknown" => "workq-pressure-unknown",
            _ => unreachable!(),
        };
        assert!(
            cx.debug_bounds(selector).is_some(),
            "{state}: expected {pressure}"
        );
        assert_eq!(
            cx.debug_bounds("workq-processing-item").is_some(),
            processing
        );
        app.read_with(&cx, |app, _| {
            assert!(
                workq_activity::observed_busy(&app.frames, 0, &key)
                    .percent()
                    .is_none()
            )
        });
        assert!(
            cx.debug_bounds("workq-activity-view-1.000-2.000").is_some(),
            "untimed views use frame order, not fabricated seconds"
        );
    }
}

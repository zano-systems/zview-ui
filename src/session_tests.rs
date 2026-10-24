use super::{Screen, open_session_menu, setup};
use gpui::{TestAppContext, px, size};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[gpui_kit::test]
fn session_menu_opens_dump_and_reloads_the_current_file(cx: &mut TestAppContext) {
    let (app, mut cx) = setup(cx);
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-data/mutexes.json");
    open_session_menu(&mut cx);
    cx.simulate_keystrokes("down down enter");
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|_| Some(vec![path.clone()]));
    cx.run_until_parked();
    app.read_with(&cx, |app, _| {
        assert!(app.error.is_none());
        assert_eq!(app.source.as_ref(), Some(&path));
        assert!(app.recording_elf.is_none());
        assert_eq!(app.frame, 0);
    });
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.step(true, window, cx);
            app.show_kernel_objects(None, window, cx);
        });
    });
    open_session_menu(&mut cx);
    cx.simulate_keystrokes("down down down down enter");
    cx.run_until_parked();
    app.read_with(&cx, |app, _| {
        assert!(app.error.is_none());
        assert_eq!(app.source.as_ref(), Some(&path));
        assert_eq!(app.frame, 0);
        assert_eq!(app.screen, Screen::Overview);
        assert!(!app.playing);
    });
}

#[gpui_kit::test]
fn session_menu_stays_in_bounds_and_dismissal_preserves_playback(cx: &mut TestAppContext) {
    let (app, mut cx) = setup(cx);
    cx.simulate_resize(size(px(1000.), px(680.)));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            // Full paths must not expand the popup beyond the window.
            app.source =
                Some(format!("/workspace/{}/snapshot.json", "long-path/".repeat(40)).into());
            app.toggle_playback(window, cx);
        });
    });
    open_session_menu(&mut cx);
    for selector in ["session-menu-trigger", "session-menu-source"] {
        let bounds = cx.debug_bounds(selector).unwrap();
        assert!(bounds.left() >= px(0.));
        assert!(bounds.right() <= px(1000.));
        assert!(bounds.bottom() <= px(680.));
    }
    cx.simulate_keystrokes("escape");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("session-menu-source").is_none());
    app.read_with(&cx, |app, _| {
        assert!(app.playing);
        assert_eq!(app.frame, 1);
        assert_eq!(app.screen, Screen::Overview);
    });
}

#[gpui_kit::test]
fn session_menu_skips_disabled_actions_and_can_cancel_import(cx: &mut TestAppContext) {
    let (app, mut cx) = setup(cx);
    let cancel = Arc::new(AtomicBool::new(false));
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.loading = Some(cancel.clone());
            cx.notify();
        });
    });
    open_session_menu(&mut cx);
    // Connect, Open dump, Open recording, and Reload are disabled.
    // After the initial heading, Down selects Cancel import, not a picker/connection.
    cx.simulate_keystrokes("down down enter");
    assert!(!cx.did_prompt_for_paths());
    assert!(cancel.load(Ordering::Relaxed));
    app.read_with(&cx, |app, _| {
        assert_eq!(app.frame, 1);
        assert!(app.source.is_none());
        assert!(app.error.is_none());
    });
}

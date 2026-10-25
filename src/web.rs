//! Browser host for the same application used by the desktop executable.
use super::*;
use gpui_kit::web::{CanvasFontFallback, WebBackendPreference, WebPlatform};
use std::{borrow::Cow, cell::RefCell, rc::Rc, sync::Arc};
use wasm_bindgen::prelude::*;

thread_local! {
    // The browser owns the event loop. Keep GPUI alive and retain the one window.
    static APPLICATION: RefCell<Option<ApplicationHandle>> = const { RefCell::new(None) };
    static VIEW: RefCell<Option<(AnyWindowHandle, Entity<ThreadsApp>)>> = const { RefCell::new(None) };
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = window, js_name = zviewOpenJson)]
    fn pick_json();
}

impl ThreadsApp {
    pub(super) fn open(&mut self, _: &ClickEvent, _: &mut Window, _: &mut Context<Self>) {
        pick_json();
    }
}

#[wasm_bindgen]
pub async fn start(ui_font: Vec<u8>, mono_font: Vec<u8>) -> Result<(), JsValue> {
    console_error_panic_hook::set_once();
    if APPLICATION.with(|app| app.borrow().is_some()) {
        return Err(JsValue::from_str("ZView is already running."));
    }
    gpui_kit::platform::web_init();
    let platform = Rc::new(WebPlatform::new_with_backend_and_font_fallback(
        false,
        WebBackendPreference::Auto,
        CanvasFontFallback::EmojiAndCjk,
    ));
    let http = Arc::new(platform.fetch_http_client());
    let app = Application::with_platform(platform)
        .with_http_client(http)
        .with_assets(app_assets::AppAssets);
    // WebPlatform prepares the GPU asynchronously before invoking this closure.
    // Do not accept an initial document until the window actually exists.
    let (ready, launched) = futures_channel::oneshot::channel();
    let handle = app.run_embedded(move |cx| {
        init(cx);
        bind_keys(cx);
        cx.text_system()
            .add_fonts(vec![Cow::Owned(ui_font), Cow::Owned(mono_font)])
            .expect("load bundled browser fonts");
        apply_theme(cx);
        Theme::update(cx, |theme| {
            theme.font_family = "IBM Plex Sans".into();
            theme.mono_font_family = "JetBrains Mono".into();
        });
        let frames = model::parse(include_str!("../sample.json")).expect("valid bundled dump");
        let view = open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| ThreadsApp::from_frames(frames, None, None, window, cx))
        })
        .expect("open ZView browser window");
        VIEW.with(|slot| *slot.borrow_mut() = Some(view));
        cx.activate(true);
        let _ = ready.send(());
    });
    APPLICATION.with(|slot| *slot.borrow_mut() = Some(handle));
    launched
        .await
        .map_err(|_| JsValue::from_str("Browser window initialization failed."))
}

/// Install browser-provided text without introducing filesystem access.
/// Invalid input leaves the previous frames and selection visible.
#[wasm_bindgen]
pub fn load_json(text: &str, name: &str) -> Result<(), JsValue> {
    let frames = if text.len() > 16 * 1024 * 1024 {
        Err("Dump exceeds the 16 MiB limit.".to_string())
    } else {
        model::parse(text)
    };
    APPLICATION.with(|slot| {
        let app = slot.borrow();
        let app = app
            .as_ref()
            .ok_or_else(|| JsValue::from_str("ZView is not running."))?;
        VIEW.with(|slot| {
            let view = slot.borrow();
            let (window, view) = view
                .as_ref()
                .ok_or_else(|| JsValue::from_str("No ZView window."))?;
            app.update(|cx| {
                window.update(cx, |_, window, cx| {
                    view.update(cx, |this, cx| {
                        match frames {
                            Ok(frames) => {
                                this.install(frames, PathBuf::from(name), None, window, cx)
                            }
                            Err(error) => this.error = Some(error),
                        }
                        cx.notify();
                    });
                })
            })
            .map_err(|error| JsValue::from_str(&error.to_string()))
        })
    })
}

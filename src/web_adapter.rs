//! HTTP/WebSocket client for the existing GPUI browser host, not a second UI.
use super::*;
use crate::adapter_protocol::{
    ClientState, Connection, Envelope, Event, LiveView, Mode, Status, ViewInterest,
};
use futures_util::StreamExt;
use gloo_net::{
    http::Request,
    websocket::{Message, futures::WebSocket},
};
use gloo_timers::future::TimeoutFuture;
use serde_json::json;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["globalThis", "crypto"], js_name = randomUUID)]
    fn browser_client_id() -> String;
}

#[path = "web_session.rs"]
mod session_ui;
#[path = "web_usage.rs"]
mod usage_ui;

pub(super) struct Controls {
    pub available: bool,
    pub state: ClientState,
    displayed: Option<(String, u64)>,
    history_view: Option<LiveView>,
    history_screen: Option<Screen>,
    last_frame_received: Option<(web_time::Instant, f64)>,
    client_id: Option<String>,
    view_interest: ViewInterest,
    usage: usage_ui::UsageInput,
    draft: session_ui::Draft,
    _draft_subscriptions: Vec<Subscription>,
}

impl Controls {
    pub fn new(window: &mut Window, cx: &mut Context<ThreadsApp>) -> Self {
        let draft = session_ui::Draft::new(window, cx);
        let subscriptions = draft
            .inputs()
            .into_iter()
            .map(|input| {
                cx.subscribe_in(&input, window, |app, _, _: &InputEvent, window, cx| {
                    app.session_draft_changed(window, cx);
                })
            })
            .collect();
        Self {
            available: false,
            state: ClientState::default(),
            displayed: None,
            history_view: None,
            history_screen: None,
            last_frame_received: None,
            client_id: None,
            view_interest: ViewInterest::default(),
            usage: usage_ui::UsageInput::new(window, cx),
            draft,
            _draft_subscriptions: subscriptions,
        }
    }
}

impl Controls {
    fn fresh_frame(&self) -> bool {
        let same_session = self.state.session.as_ref().is_some_and(|session| {
            self.displayed
                .as_ref()
                .is_some_and(|(id, _)| *id == session.id)
        });
        same_session
            && self.last_frame_received.is_some_and(|(received, age)| {
                age.is_finite() && age + received.elapsed().as_secs_f64() < 5.
            })
    }

    fn indicator(&self) -> crate::adapter_protocol::SessionIndicator {
        self.state.indicator(self.fresh_frame())
    }
}

impl ThreadsApp {
    fn set_usage_window(&mut self, polls: u32, cx: &mut Context<Self>) {
        if let (Some(session), Some(client_id)) =
            (&self.adapter.state.session, &self.adapter.client_id)
        {
            self.adapter_command(
                "/api/session/usage-window",
                json!({"session_id": session.id, "client_id": client_id, "polls": polls}),
                cx,
            );
        }
    }

    fn desired_live_view(&self, cx: &Context<Self>) -> LiveView {
        let label = self.heap.read(cx).selected_value();
        let heap = self
            .adapter
            .state
            .session
            .as_ref()
            .and_then(|s| s.inventory.as_ref())
            .and_then(|inventory| inventory.get("heaps"))
            .and_then(|rows| {
                rows.iter()
                    .find(|row| Some(&format!("{} · {:#x}", row.name, row.address)) == label)
            })
            .cloned();
        self.screen.live_view(
            heap,
            self.object_kind,
            self.object_search.read(cx).value().to_string(),
        )
    }

    pub(crate) fn live_view_loading(&self, cx: &Context<Self>) -> bool {
        let Some(session) = self.adapter.state.session.as_ref() else {
            return false;
        };
        if session.mode() != Mode::Live
            || session.status != Status::Reading
            || session.inventory.is_none()
        {
            return false;
        }
        if self.live_history.is_empty() {
            return true;
        }
        let Some(view) = &session.live_view else {
            return true;
        };
        view.owner.as_ref() != self.adapter.client_id.as_ref()
            || session.last_frame.as_ref().is_none_or(|frame| {
                frame.view_revision != view.revision
                    || frame.view.as_ref() != Some(&self.desired_live_view(cx))
            })
    }

    pub(crate) fn live_view_loading_note(&self) -> String {
        if self
            .adapter
            .state
            .session
            .as_ref()
            .and_then(|s| s.live_view.as_ref())
            .is_some_and(|view| {
                view.owner.is_some() && view.owner.as_ref() != self.adapter.client_id.as_ref()
            })
        {
            return "Another tab controls Live acquisition. Close it before controlling this view."
                .into();
        }
        self.adapter
            .view_interest
            .error
            .clone()
            .unwrap_or_else(|| "Reading the selected view…".into())
    }

    /// The current screen owns one selection and optional detail request.
    pub(crate) fn sync_live_view(&mut self, cx: &mut Context<Self>) {
        let binding = self
            .adapter
            .state
            .session
            .as_ref()
            .filter(|session| {
                session.mode() == Mode::Live
                    && session.status == Status::Reading
                    && self.adapter.state.connection == Connection::Connected
            })
            .map(|session| session.id.as_str());
        self.adapter.view_interest.bind(binding);
        if self.adapter.state.is_live() {
            let desired = self.desired_live_view(cx);
            if self.adapter.history_screen.as_ref() != Some(&self.screen) {
                self.live_history.clear();
                self.adapter.history_screen = Some(self.screen.clone());
            }
            self.adapter.history_view = Some(desired);
        }
        let Some(session) = self.adapter.state.session.as_ref() else {
            return;
        };
        if session.mode() != Mode::Live
            || session.status != Status::Reading
            || self.adapter.state.connection != Connection::Connected
            || session.inventory.is_none()
        {
            return;
        }
        let Some(client_id) = self.adapter.client_id.clone() else {
            return;
        };
        if session
            .live_view
            .as_ref()
            .is_some_and(|view| view.owner.is_some() && view.owner.as_ref() != Some(&client_id))
        {
            return;
        }
        let view = self.desired_live_view(cx);
        let Some(epoch) = self.adapter.view_interest.begin(
            Some(view.clone()),
            session
                .live_view
                .as_ref()
                .and_then(|state| state.active_view()),
        ) else {
            return;
        };
        let session_id = session.id.clone();
        spawn_local(async move {
            let result =
                async {
                    let response = Request::post("/api/session/view")
                    .json(&json!({"session_id": session_id, "client_id": client_id, "view": view}))
                    .map_err(|e| e.to_string())?.send().await.map_err(|e| e.to_string())?;
                    if response.ok() {
                        return Ok((None, false));
                    }
                    let text = response.text().await.map_err(|e| e.to_string())?;
                    let data = serde_json::from_str::<serde_json::Value>(&text).ok();
                    let conflict = data.as_ref().is_some_and(|v| v["code"] == "view_busy");
                    let error = data
                        .and_then(|v| v["error"].as_str().map(str::to_owned))
                        .unwrap_or(text);
                    Ok((Some(error), conflict))
                }
                .await;
            let _ = web::update_view(|this, _, cx| {
                let (error, conflict) = result.unwrap_or_else(|e| (Some(e), false));
                this.adapter.view_interest.finish(epoch, error, conflict);
                cx.notify();
            });
        });
    }

    pub(crate) fn live_heap_map(
        &self,
        key: &model::HeapKey,
    ) -> Option<&crate::adapter_protocol::CapturedHeapMap> {
        let session = self.adapter.state.session.as_ref()?;
        if session.mode() != Mode::Live
            || self
                .adapter
                .displayed
                .as_ref()
                .is_none_or(|(id, _)| *id != session.id)
            || key.in_frame(&self.frames[self.frame]).is_none()
        {
            return None;
        }
        if session.status == Status::Reading
            && self.adapter.state.connection == Connection::Connected
            && session.live_view.as_ref().is_none_or(|view| {
                view.owner.as_ref() != self.adapter.client_id.as_ref()
                    || session
                        .last_frame
                        .as_ref()
                        .is_none_or(|frame| frame.view_revision != view.revision)
            })
        {
            return None;
        }
        if !matches!(session.last_frame.as_ref().and_then(|frame| frame.view.as_ref()),
            Some(LiveView::Detail { group, object })
                if group == "heaps" && object.address == key.address && object.name == key.name)
        {
            return None;
        }
        let origin = self.live_history.origin()?;
        session.heap_detail.last_map.as_ref().filter(|map| {
            map.heap.address == key.address
                && map.heap.name == key.name
                && map.live_elapsed >= origin
        })
    }

    pub(crate) fn live_heap_note(&self, key: &model::HeapKey) -> Option<String> {
        let session = self.adapter.state.session.as_ref()?;
        if session.mode() != Mode::Live {
            return None;
        }
        if self.adapter.state.connection != Connection::Connected
            || session.status != Status::Reading
        {
            return Some("Retained heap map · acquisition is not updating this view.".into());
        }
        if let Some(error) = &self.adapter.view_interest.error {
            return Some(error.clone());
        }
        let detail = &session.heap_detail;
        if let Some(heap) = &detail.heap
            && (heap.address != key.address || heap.name != key.name)
        {
            return Some(
                "Another tab is inspecting a different heap. Close that heap detail first.".into(),
            );
        }
        if let Some(error) = &detail.error {
            return Some(error.clone());
        }
        if let Some(map) = self.live_heap_map(key) {
            return Some(format!(
                "Last captured map at {:.2}s since opening this view · {} Read time limits the achieved rate.",
                map.live_elapsed - self.live_history.origin().unwrap_or(map.live_elapsed),
                session
                    .timing
                    .as_ref()
                    .map(|timing| timing.heap_label())
                    .unwrap_or_else(|| "Requested map timing unavailable.".into()),
            ));
        }
        Some("Collecting heap map…".into())
    }

    fn stop_adapter(&mut self, cx: &mut Context<Self>) {
        if self.adapter.state.can_stop()
            && let Some(session) = &self.adapter.state.session
        {
            self.adapter_command("/api/session/stop", json!({"session_id": session.id}), cx);
        }
    }

    fn reconnect_adapter(&mut self, cx: &mut Context<Self>) {
        if self.adapter.state.can_reconnect()
            && let Some(session) = &self.adapter.state.session
        {
            self.adapter_command(
                "/api/session/reconnect",
                json!({"session_id": session.id}),
                cx,
            );
        }
    }

    fn adapter_command(
        &mut self,
        path: &'static str,
        body: serde_json::Value,
        cx: &mut Context<Self>,
    ) {
        self.adapter.state.pending = true;
        self.error = None;
        cx.notify();
        spawn_local(async move {
            let result = async {
                let response = Request::post(path)
                    .json(&body)
                    .map_err(|error| error.to_string())?
                    .send()
                    .await
                    .map_err(|error| error.to_string())?;
                if response.ok() {
                    return Ok(());
                }
                let text = response.text().await.map_err(|error| error.to_string())?;
                Err(serde_json::from_str::<serde_json::Value>(&text)
                    .ok()
                    .and_then(|value| value["error"].as_str().map(str::to_owned))
                    .unwrap_or(text))
            }
            .await;
            let _ = web::update_view(|this, window, cx| {
                this.adapter.state.pending = false;
                if let Err(error) = result {
                    this.adapter.draft.error = Some(error.clone());
                    this.error = Some(error);
                    if path == "/api/session/usage-window" {
                        this.adapter.usage.restore(window, cx);
                    }
                } else if !matches!(path, "/api/session/stop" | "/api/session/usage-window")
                    && this.adapter.draft.open
                {
                    this.adapter.draft.dismiss();
                    window.close_dialog(cx);
                }
                // HTTP is an acknowledgement; only the stream replaces state.
                cx.notify();
            });
        });
    }

    fn accept_adapter(
        &mut self,
        message: Envelope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let interrupted = matches!(
            &message.event,
            Event::Notice { .. } | Event::View(_) | Event::Status(_) | Event::UsageWindow { .. }
        );
        if !self.adapter.state.accept(message)? {
            return Ok(());
        }
        self.adapter
            .usage
            .sync(self.adapter.state.session.as_ref(), window, cx);
        if interrupted {
            self.live_history.interrupt();
        }
        if let Some(session) = &self.adapter.state.session {
            let key = (session.id.clone(), session.frames_received);
            let is_live = session.mode() == Mode::Live;
            if let Some(frame) = &session.last_frame
                && self.adapter.displayed.as_ref() != Some(&key)
            {
                // Reuse the shared model, its validations and all existing views.
                // Parse the measurements first: a reconnect snapshot may start
                // after zero. Only Live gets monotonic receipt-time chart axes.
                let text = serde_json::to_string(&frame.groups).map_err(|e| e.to_string())?;
                let mut frames = model::parse(&text)?;
                if session.mode() == Mode::Live {
                    frames[0].elapsed_seconds = frame.live_elapsed;
                    frames[0].is_live = true;
                }
                let new_session =
                    self.adapter.displayed.as_ref().map(|key| &key.0) != Some(&session.id);
                let source = PathBuf::from(session.source_label());
                let elf = PathBuf::from(&session.elf);
                // Snapshot recovery must not make old measurements look fresh.
                let source_age = web_time::SystemTime::now()
                    .duration_since(web_time::UNIX_EPOCH)
                    .map(|now| (now.as_secs_f64() - frame.decoded_at).max(0.))
                    .unwrap_or(f64::NAN);
                let received_view = frame.view.clone();
                let overview = matches!(&received_view, None | Some(LiveView::Overview { .. }));
                if new_session {
                    let recording_elf = (session.mode() == Mode::Replay).then_some(elf);
                    self.install(frames, source, recording_elf, window, cx);
                    self.adapter.history_view = None;
                    self.adapter.history_screen = None;
                } else if is_live {
                    self.pause(cx);
                    self.frames = frames;
                    self.frame = 0;
                    self.sync_heaps(window, cx);
                } else {
                    self.pause(cx);
                    if frames[0].elapsed_seconds.is_some_and(|time| {
                        self.frames
                            .last()
                            .and_then(|f| f.elapsed_seconds)
                            .is_some_and(|last| time <= last)
                    }) {
                        return Err("Live receipt times must increase within a session.".into());
                    }
                    self.frames.append(&mut frames);
                    if self.frames.len() > 256 {
                        self.frames.remove(0);
                    }
                    self.frame = self.frames.len() - 1;
                    self.sync_heaps(window, cx);
                }
                if is_live
                    && self.adapter.history_view.is_some()
                    && self.adapter.history_view == received_view
                {
                    self.live_history.record(&self.frames[self.frame], overview);
                }
                self.adapter.last_frame_received = Some((web_time::Instant::now(), source_age));
                self.adapter.displayed = Some(key);
            }
        }
        self.error = self.adapter.state.error.clone();
        cx.notify();
        Ok(())
    }

    pub(crate) fn adapter_display_notice(&self) -> Option<AnyElement> {
        let session = self.adapter.state.session.as_ref()?;
        let previous = self
            .adapter
            .displayed
            .as_ref()
            .is_none_or(|(id, _)| *id != session.id);
        let text = if self.adapter.state.connection != Connection::Connected {
            "Adapter disconnected · displayed measurements are retained, not confirmed current."
                .to_owned()
        } else if previous {
            "Showing previous data until the selected session provides its first valid frame."
                .to_owned()
        } else if session.mode() == Mode::Live
            && session.status != crate::adapter_protocol::Status::Reading
        {
            format!("Retained live measurements · {}", session.status_label())
        } else {
            return None;
        };
        Some(layout::note("adapter-display-notice", text).into_any_element())
    }
}

/// Called once by the existing browser host. Static JSON hosting still works:
/// no adapter API (404) means no controls and no reconnect loop.
#[wasm_bindgen]
pub fn connect_adapter() {
    spawn_local(async {
        match Request::get("/api/session").send().await {
            Ok(response) if response.status() == 404 => return,
            Ok(response) if response.ok() => {}
            _ => return,
        }
        let _ = web::update_view(|this, _, cx| {
            this.adapter.available = true;
            cx.notify();
        });
        // Freshness changes even when neither frames nor health events arrive.
        // This is one UI-only timer, never another acquisition loop.
        spawn_local(async {
            loop {
                TimeoutFuture::new(1000).await;
                if web::update_view(|this, _, cx| {
                    if this
                        .adapter
                        .state
                        .session
                        .as_ref()
                        .is_some_and(|s| s.status == crate::adapter_protocol::Status::Reading)
                    {
                        cx.notify();
                    }
                })
                .is_err()
                {
                    break;
                }
            }
        });
        let location = web_sys::window().expect("browser window").location();
        let scheme = if location.protocol().unwrap_or_default() == "https:" {
            "wss"
        } else {
            "ws"
        };
        let url = format!(
            "{scheme}://{}/api/events",
            location.host().unwrap_or_default()
        );
        loop {
            let client_id = browser_client_id();
            let _ = web::update_view(|this, _, _| {
                this.adapter.client_id = Some(client_id.clone());
                this.adapter.view_interest.reset();
            });
            if let Ok(mut socket) = WebSocket::open(&format!("{url}?client_id={client_id}")) {
                let mut needs_snapshot = true;
                while let Some(message) = socket.next().await {
                    let result = match message {
                        Ok(Message::Text(text)) => serde_json::from_str::<Envelope>(&text)
                            .map_err(|error| error.to_string())
                            .and_then(|message| {
                                if needs_snapshot && !matches!(message.event, Event::Snapshot(_)) {
                                    return Err("Expected adapter snapshot.".into());
                                }
                                let mut accepted = Ok(());
                                web::update_view(|this, window, cx| {
                                    accepted = this.accept_adapter(message, window, cx);
                                })
                                .map_err(|_| "Browser window is unavailable.".to_owned())?;
                                accepted
                            }),
                        Ok(Message::Bytes(_)) => Err("Expected JSON text from adapter.".into()),
                        Err(_) => break,
                    };
                    if let Err(error) = result {
                        let _ = web::update_view(|this, _, cx| {
                            this.error = Some(format!("Adapter protocol error: {error}"));
                            cx.notify();
                        });
                        break;
                    }
                    needs_snapshot = false;
                }
            }
            let _ = web::update_view(|this, _, cx| {
                this.adapter.state.connection = Connection::Retrying;
                this.adapter.client_id = None;
                this.adapter.view_interest.reset();
                this.live_history.interrupt();
                cx.notify();
            });
            TimeoutFuture::new(1500).await;
        }
    });
}

/// Use the same Rust command path as the GPUI controls; useful to browser hosts.
#[wasm_bindgen]
pub fn adapter_open_replay(recording: &str, elf: &str) -> Result<(), JsValue> {
    web::update_view(|this, _, cx| {
        if this.adapter.state.can_open() {
            this.adapter_command(
                "/api/session/replay",
                json!({
                    "recording": recording, "elf": elf,
                }),
                cx,
            );
        }
    })
}

#[wasm_bindgen]
pub fn adapter_stop() -> Result<(), JsValue> {
    web::update_view(|this, _, cx| this.stop_adapter(cx))
}

#[wasm_bindgen]
pub fn adapter_reconnect() -> Result<(), JsValue> {
    web::update_view(|this, _, cx| this.reconnect_adapter(cx))
}

/// Read-only inspection of Rust state, including measurements in the shared UI.
/// Canvas rendering has no DOM table for browser tests to inspect.
#[wasm_bindgen]
pub fn adapter_state() -> Result<String, JsValue> {
    let mut state = serde_json::Value::Null;
    web::update_view(|this, _, cx| {
        let adapter = &this.adapter;
        state = json!({
            "available": adapter.available,
            "screen": format!("{:?}", this.screen),
            "connected": adapter.state.connection == Connection::Connected,
            "status": adapter.state.session.as_ref().map(|s| s.status.as_str()),
            "mode": adapter.state.session.as_ref().map(|s| s.mode()),
            "samples_per_second": adapter.state.session.as_ref().and_then(|s| s.samples_per_second),
            "timing": adapter.state.session.as_ref().and_then(|s| s.timing.as_ref()),
            "live_elapsed": this.frames[this.frame].is_live.then_some(this.frames[this.frame].elapsed_seconds),
            "displayed_session": adapter.displayed.as_ref().map(|(id, _)| id),
            "session_id": adapter.state.session.as_ref().map(|s| &s.id),
            "frames_received": adapter.state.session.as_ref().map(|s| s.frames_received),
            "buffered_frames": this.frames.len(),
            "history_changes": this.live_history.changes(),
            "history_bytes": this.live_history.bytes(),
            "history_seconds": this.live_history.retained_seconds(),
            "history_label": this.live_history.label(),
            "heap_capture": adapter.state.session.as_ref().and_then(|s| s.heap_detail.heap.as_ref()),
            "heap_map_chunks": adapter.state.session.as_ref().and_then(|s| s.heap_detail.last_map.as_ref()).map(|m| m.chunks.len()),
            "displayed_heap_map_chunks": match &this.screen {
                Screen::Heap(key) => this.live_heap_map(key).map(|map| map.chunks.len()),
                _ => None,
            },
            "heap_map_frame": adapter.state.session.as_ref().and_then(|s| s.heap_detail.last_map.as_ref()).map(|m| m.frame_number),
            "heap_map_note": match &this.screen { Screen::Heap(key) => this.live_heap_note(key), _ => None },
            "error": this.error,
            "visible_threads": this.frames[this.frame].visible_threads().len(),
            "cpu": this.frames[this.frame].cpu(),
            "source": this.source,
            "can_open": adapter.state.can_open(),
            "can_stop": adapter.state.can_stop(),
            "can_reconnect": adapter.state.can_reconnect(),
            "primary_action": adapter.state.primary_action().label(),
            "primary_enabled": adapter.state.can_primary_action(),
            "indicator": adapter.indicator().as_str(),
            "sidebar_collapsed": this.collapsed,
            "session_labels": crate::adapter_protocol::session_labels(
                adapter.state.session.as_ref(), adapter.state.connection),
            "context": adapter.state.session.as_ref().and_then(|s| s.context.clone()),
            "dialog_open": adapter.draft.open,
            "draft_mode": adapter.draft.mode,
            "draft_error": adapter.draft.error,
            "build_context": adapter.draft.context(cx),
            "draft_request": adapter.draft.open.then(|| adapter.draft.request(cx)),
            "draft_recording": adapter.draft.recording_path(cx),
            "native_picker": adapter.draft.picker,
        });
        state["history_origin"] = json!(this.live_history.origin());
        state["history_view"] = json!(adapter.history_view);
        state["usage_window"] = json!(adapter.state.session.as_ref().and_then(|s| s.usage_window));
        state["usage_input"] = json!(adapter.usage.state.read(cx).value().to_string());
        let input = adapter.usage.state.read(cx).input_bounds();
        state["usage_input_bounds"] = json!({
            "x": input.left().as_f32(), "y": input.top().as_f32(),
            "width": input.size.width.as_f32(), "height": input.size.height.as_f32(),
        });
        state["requested_usage_window"] = json!(
            adapter
                .state
                .session
                .as_ref()
                .and_then(|s| s.live_view.as_ref())
                .map(|v| v.usage_window)
        );
        state["draft_advanced"] = json!(adapter.draft.advanced);
        state["view_loading"] = json!(this.live_view_loading(cx));
        state["view_error"] = json!(adapter.view_interest.error);
        state["requested_view"] = json!(this.desired_live_view(cx));
        state["sampled_view"] = json!(
            adapter
                .state
                .session
                .as_ref()
                .and_then(|s| s.last_frame.as_ref())
                .and_then(|f| f.view.as_ref())
        );
        state["sampled_groups"] = json!(
            adapter
                .state
                .session
                .as_ref()
                .and_then(|s| s.last_frame.as_ref())
                .map(|f| &f.sampled_groups)
        );
        state["controls_view"] = json!(
            adapter
                .state
                .session
                .as_ref()
                .and_then(|s| s.live_view.as_ref())
                .is_some_and(|view| view.owner.as_ref() == adapter.client_id.as_ref())
        );
    })?;
    Ok(state.to_string())
}

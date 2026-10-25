mod app_assets;
mod chart_zoom;
mod heap_detail;
mod heap_map;
mod kernel_model;
mod kernel_objects;
mod layout;
mod model;
mod msgq_detail;
mod mutex_detail;
#[cfg(not(target_family = "wasm"))]
mod recording;
mod semaphore_detail;
mod session;
mod slab_detail;
mod step_area_chart;
#[cfg(test)]
mod tests;
mod thread_detail;
mod time_series_chart;
mod wait_queue;
#[cfg(target_family = "wasm")]
mod web;
mod workq_activity;
mod workq_detail;
mod zoom_strip;

use gpui_kit::component::InteractiveElementExt;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, IndexPath, Sizable, StyledExt, Theme, ThemeMode,
    WindowExt,
    button::{Button, ButtonVariants},
    input::{InputEvent, InputState},
    notification::{Notification, NotificationDelivery},
    progress::Progress,
    scroll::ScrollableElement,
    select::{SearchableVec, Select, SelectEvent, SelectState},
    sidebar::{
        Sidebar, SidebarFooter, SidebarGroup, SidebarHeader, SidebarMenu, SidebarMenuItem,
        SidebarToggleButton,
    },
    tooltip::Tooltip,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use model::{Frame, ThreadKey, percent, percentage};
use std::path::PathBuf;

actions!(
    threads,
    [
        OpenThread,
        OpenSemaphore,
        OpenMessageQueue,
        OpenMemorySlab,
        OpenMutex,
        OpenWorkQueue,
        OpenHeap,
        BackToThreads,
        BackToObjects,
        SelectNextThread,
        SelectPreviousThread
    ]
);

const BG: u32 = 0x1e1d34;
const CARD: u32 = 0x25223b;
const BORDER: u32 = 0x39324e;
const MUTED: u32 = 0xa59bb8;
const PURPLE: u32 = 0xac91f5;
const GREEN: u32 = 0x69cbb1;
const BLUE: u32 = 0x75b3f1;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Screen {
    Overview,
    Thread(ThreadKey),
    KernelObjects,
    Semaphore(kernel_model::SemaphoreKey),
    MessageQueue(kernel_model::MessageQueueKey),
    MemorySlab(kernel_model::MemorySlabKey),
    Mutex(kernel_model::MutexKey),
    WorkQueue(kernel_model::WorkQueueKey),
    Heap(model::HeapKey),
}

impl Screen {
    fn object_kind(&self) -> Option<kernel_model::Kind> {
        use kernel_model::Kind;
        match self {
            Self::Semaphore(_) => Some(Kind::Semaphore),
            Self::MessageQueue(_) => Some(Kind::MessageQueue),
            Self::MemorySlab(_) => Some(Kind::MemorySlab),
            Self::Mutex(_) => Some(Kind::Mutex),
            Self::WorkQueue(_) => Some(Kind::WorkQueue),
            Self::Heap(_) => Some(Kind::Heap),
            Self::Overview | Self::Thread(_) | Self::KernelObjects => None,
        }
    }

    fn name(&self) -> Option<&str> {
        // Retain the route identity even when the object is absent in a frame.
        match self {
            Self::Thread(key) => Some(&key.name),
            Self::Semaphore(key) => Some(&key.name),
            Self::MessageQueue(key) => Some(&key.name),
            Self::MemorySlab(key) => Some(&key.name),
            Self::Mutex(key) => Some(&key.name),
            Self::WorkQueue(key) => Some(&key.name),
            Self::Heap(key) => Some(&key.name),
            Self::Overview | Self::KernelObjects => None,
        }
    }
}

struct MissingObjectNotification;

struct ThreadsApp {
    frames: Vec<Frame>,
    frame: usize,
    playing: bool,
    playback_task: Option<Task<()>>,
    playback_anchor: Option<(web_time::Instant, f64)>,
    playback_position: Option<f64>,
    source: Option<PathBuf>,
    recording_elf: Option<PathBuf>,
    loading: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    error: Option<String>,
    collapsed: bool,
    selected: Option<u64>,
    screen: Screen,
    missing_notice: Option<Screen>,
    table_focus: FocusHandle,
    detail_focus: FocusHandle,
    thread_rows_scroll: ScrollHandle,
    thread_columns_scroll: ScrollHandle,
    object_rows_scroll: ScrollHandle,
    object_columns_scroll: ScrollHandle,
    heap_map_states: Entity<heap_map::MapStates>,
    workq_activity_states: Entity<workq_activity::States>,
    heap: Entity<SelectState<SearchableVec<String>>>,
    _heap_subscription: Subscription,
    object_search: Entity<InputState>,
    object_kind: Option<kernel_model::Kind>,
    _object_search_subscription: Subscription,
}

impl Drop for ThreadsApp {
    fn drop(&mut self) {
        if let Some(cancel) = &self.loading {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

impl ThreadsApp {
    #[cfg(not(target_family = "wasm"))]
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let source = std::env::args_os().nth(1).map(PathBuf::from);
        Self::from_source(source, window, cx)
    }

    #[cfg(not(target_family = "wasm"))]
    fn from_source(source: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let loaded = source.as_deref().map(model::load);
        let error = loaded.as_ref().and_then(|r| r.as_ref().err().cloned());
        let frames = loaded.and_then(Result::ok).unwrap_or_else(|| {
            model::parse(include_str!("../sample.json")).expect("valid bundled dump")
        });
        Self::from_frames(
            frames,
            if error.is_none() { source } else { None },
            error,
            window,
            cx,
        )
    }

    fn from_frames(
        frames: Vec<Frame>,
        source: Option<PathBuf>,
        error: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let heap = cx.new(|cx| {
            SelectState::new(SearchableVec::new(Vec::<String>::new()), None, window, cx)
                .searchable(true)
        });
        let subscription = cx.subscribe_in(
            &heap,
            window,
            |_, _, _: &SelectEvent<SearchableVec<String>>, _, cx| cx.notify(),
        );
        let object_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search name or address…"));
        let object_search_subscription =
            cx.subscribe_in(&object_search, window, |_, _, _: &InputEvent, _, cx| {
                cx.notify()
            });
        let mut app = Self {
            frame: 0,
            playing: false,
            playback_task: None,
            playback_anchor: None,
            playback_position: None,
            frames,
            source,
            recording_elf: None,
            loading: None,
            error,
            collapsed: false,
            selected: None,
            screen: Screen::Overview,
            missing_notice: None,
            table_focus: cx.focus_handle(),
            detail_focus: cx.focus_handle(),
            thread_rows_scroll: ScrollHandle::new(),
            thread_columns_scroll: ScrollHandle::new(),
            object_rows_scroll: ScrollHandle::new(),
            object_columns_scroll: ScrollHandle::new(),
            heap_map_states: cx.new(|_| heap_map::MapStates::default()),
            workq_activity_states: cx.new(|_| workq_activity::States::default()),
            heap,
            _heap_subscription: subscription,
            object_search,
            object_kind: None,
            _object_search_subscription: object_search_subscription,
        };
        app.sync_heaps(window, cx);
        app
    }

    fn sync_heaps(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let old = self.heap.read(cx).selected_value().cloned();
        let items: Vec<_> = self.frames[self.frame]
            .heaps
            .iter()
            .map(|h| h.label())
            .collect();
        let index = old
            .and_then(|old| items.iter().position(|label| *label == old))
            .or_else(|| (!items.is_empty()).then_some(0));
        self.heap.update(cx, |state, cx| {
            state.set_items(SearchableVec::new(items), window, cx);
            state.set_selected_index(index.map(|i| IndexPath::default().row(i)), window, cx);
        });
    }

    #[cfg(test)]
    fn load(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        match model::load(&path) {
            Ok(frames) => self.install(frames, path, None, window, cx),
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }

    fn install(
        &mut self,
        frames: Vec<Frame>,
        path: PathBuf,
        elf: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pause(cx);
        self.playback_position = None;
        self.frame = 0;
        self.frames = frames;
        self.source = Some(path);
        self.recording_elf = elf;
        self.error = None;
        self.selected = None;
        self.screen = Screen::Overview;
        self.heap_map_states.update(cx, |states, _| states.clear());
        self.workq_activity_states
            .update(cx, |states, _| states.clear());
        self.thread_rows_scroll.set_offset(point(px(0.), px(0.)));
        self.object_rows_scroll.set_offset(point(px(0.), px(0.)));
        self.sync_heaps(window, cx);
    }

    #[cfg(not(target_family = "wasm"))]
    fn start_load(&mut self, path: PathBuf, elf: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.loading.is_some() {
            return;
        }
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.loading = Some(cancel.clone());
        self.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn({
                    let path = path.clone();
                    let elf = elf.clone();
                    let cancel = cancel.clone();
                    async move {
                        match elf {
                            Some(elf) => recording::load(&path, &elf, cancel),
                            None => model::load(&path),
                        }
                    }
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                // Cancellation wins even if the decoder has just finished.
                let cancelled = cancel.load(std::sync::atomic::Ordering::Relaxed);
                this.loading = None;
                if !cancelled {
                    match result {
                        Ok(frames) => this.install(frames, path, elf, window, cx),
                        Err(error) => this.error = Some(error),
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    #[cfg(not(target_family = "wasm"))]
    fn open_recording(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.loading.is_some() {
            return;
        }
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Open ZView recording (.ndjson.gz)".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await
                && let Some(path) = paths.into_iter().next()
            {
                let elf_paths = cx.update(|cx| {
                    cx.prompt_for_paths(PathPromptOptions {
                        files: true,
                        directories: false,
                        multiple: false,
                        prompt: Some("Select the matching Zephyr ELF".into()),
                    })
                });
                if let Ok(Ok(Some(paths))) = elf_paths.await
                    && let Some(elf) = paths.into_iter().next()
                {
                    let _ = this.update(cx, |this, cx| this.start_load(path, Some(elf), cx));
                }
            }
        })
        .detach();
    }

    #[cfg(not(target_family = "wasm"))]
    fn open(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.loading.is_some() {
            return;
        }
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Open ZView dump JSON".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update(cx, |this, cx| this.start_load(path, None, cx));
            }
        })
        .detach();
    }

    fn sync_missing_notice(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let frame = &self.frames[self.frame];
        let missing = match &self.screen {
            Screen::Mutex(key) if key.in_frame(frame).is_none() => Some(self.screen.clone()),
            Screen::WorkQueue(key) if key.in_frame(frame).is_none() => Some(self.screen.clone()),
            Screen::Semaphore(key) if key.in_frame(frame).is_none() => Some(self.screen.clone()),
            Screen::Heap(key) if key.in_frame(frame).is_none() => Some(self.screen.clone()),
            _ => None,
        };
        if self.missing_notice == missing {
            return;
        }
        self.missing_notice = missing.clone();
        // Notification updates invalidate Root. Defer them outside the render
        // pass and only schedule a change when absence/identity changes.
        window.defer(cx, move |window, cx| {
            let (kind, name) = match missing {
                Some(Screen::Mutex(key)) => ("Mutex", key.name),
                Some(Screen::WorkQueue(key)) => ("Work queue", key.name),
                Some(Screen::Semaphore(key)) => ("Semaphore", key.name),
                Some(Screen::Heap(key)) => ("Heap", key.name),
                _ => {
                    window.remove_notification::<MissingObjectNotification>(cx);
                    return;
                }
            };
            window.push_notification(
                Notification::warning(format!(
                    "{name} is not present in the selected frame. Move to another frame or return to Kernel objects. Current values are not carried forward."
                ))
                .id::<MissingObjectNotification>()
                .title(format!("{kind} unavailable in this frame"))
                .autohide(false)
                .delivery(NotificationDelivery::InApp),
                cx,
            );
        });
    }

    fn step(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.pause(cx);
        self.playback_position = None;
        self.frame = if forward {
            (self.frame + 1).min(self.frames.len() - 1)
        } else {
            self.frame.saturating_sub(1)
        };
        self.sync_heaps(window, cx);
        cx.notify();
    }

    fn playback_time(&self, cx: &App) -> Option<f64> {
        self.playback_anchor
            .map(|(started, source)| {
                let elapsed = cx
                    .background_executor()
                    .now()
                    .duration_since(started)
                    .as_secs_f64();
                (source + elapsed).min(
                    self.frames
                        .last()
                        .and_then(|f| f.elapsed_seconds)
                        .unwrap_or(source),
                )
            })
            .or(self.playback_position)
    }

    fn pause(&mut self, cx: &App) {
        self.playback_position = self.playback_time(cx);
        self.playback_anchor = None;
        self.playing = false;
        self.playback_task = None;
    }

    fn reset_playback(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pause(cx);
        self.playback_position = None;
        self.frame = 0;
        self.sync_heaps(window, cx);
        cx.notify();
    }

    /// Schedule against source-time deadlines, not the previous render, so
    /// processing latency does not accumulate into slower playback.
    fn playback_tick(
        &mut self,
        seconds: f64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<std::time::Duration> {
        let frame = model::frame_at(&self.frames, seconds);
        if self.frame != frame {
            self.frame = frame;
            self.sync_heaps(window, cx);
            cx.notify();
        }
        let Some(next) = self.frames.get(frame + 1).and_then(|f| f.elapsed_seconds) else {
            self.playing = false;
            self.playback_position = self.frames[frame].elapsed_seconds;
            self.playback_anchor = None;
            cx.notify();
            return None;
        };
        // Sub-nanosecond float rounding must not create a ready zero-duration
        // timer that repeatedly polls the same timestamp without yielding.
        // Bound long waits before conversion: valid finite source timestamps can
        // exceed Duration or platform timer limits. Each wakeup recomputes the
        // deadline against the source-time anchor, without inventing frames.
        Some(std::time::Duration::from_secs_f64(
            (next - seconds).clamp(1e-9, 1.),
        ))
    }

    fn toggle_playback(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.playing {
            self.pause(cx);
            cx.notify();
            return;
        }
        if self.frames.len() < 2 || self.frames[0].elapsed_seconds.is_none() {
            return;
        }
        if self.frame + 1 == self.frames.len() {
            self.reset_playback(window, cx);
        }
        self.playing = true;
        let source_start = self
            .playback_position
            .unwrap_or(self.frames[self.frame].elapsed_seconds.unwrap());
        let executor = cx.background_executor().clone();
        let started = executor.now();
        self.playback_anchor = Some((started, source_start));
        let mut delay = self.playback_tick(source_start, window, cx).unwrap();
        self.playback_task = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                executor.timer(delay).await;
                let seconds = source_start + executor.now().duration_since(started).as_secs_f64();
                match this.update_in(cx, |this, window, cx| {
                    this.playback_tick(seconds, window, cx)
                }) {
                    Ok(Some(next)) => delay = next,
                    _ => break,
                }
            }
        }));
        cx.notify();
    }

    fn show_overview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.screen = Screen::Overview;
        self.table_focus.focus(window, cx);
        cx.notify();
    }

    fn open_selected(&mut self, _: &OpenThread, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(thread) = self.frames[self.frame]
            .visible_threads()
            .into_iter()
            .find(|thread| Some(thread.address) == self.selected)
        {
            self.screen = Screen::Thread(thread.key());
            self.detail_focus.focus(window, cx);
            cx.notify();
        }
    }

    fn select_thread(&mut self, forward: bool, cx: &mut Context<Self>) {
        let rows = self.frames[self.frame].visible_threads();
        if rows.is_empty() {
            self.selected = None;
        } else {
            let current = rows.iter().position(|t| Some(t.address) == self.selected);
            let index = match (current, forward) {
                (Some(i), true) => (i + 1).min(rows.len() - 1),
                (Some(i), false) => i.saturating_sub(1),
                (None, _) => 0,
            };
            self.selected = Some(rows[index].address);
            self.thread_rows_scroll.scroll_to_item(index);
        }
        cx.notify();
    }

    fn show_kernel_objects(
        &mut self,
        kind: Option<kernel_model::Kind>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.screen = Screen::KernelObjects;
        self.object_kind = kind;
        self.object_search
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn follow_object_breadcrumb(
        &mut self,
        kind: Option<kernel_model::Kind>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Breadcrumbs name destinations; unlike Back, they do not restore filters.
        self.object_search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.show_kernel_objects(kind, window, cx);
    }

    fn breadcrumbs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let is_detail = self.screen.name().is_some();
        let in_threads = matches!(self.screen, Screen::Overview | Screen::Thread(_));
        let kind = if self.screen == Screen::KernelObjects {
            self.object_kind
        } else {
            self.screen.object_kind()
        };
        let section_link = is_detail || kind.is_some();
        div()
            .debug_selector(|| "breadcrumbs".into())
            .flex()
            .flex_wrap()
            .min_w_0()
            .items_center()
            .gap_2()
            .text_sm()
            .text_color(rgb(MUTED))
            .child("Monitor")
            .child("›")
            .when(!section_link, |el| {
                el.child(if in_threads {
                    "Threads"
                } else {
                    "Kernel objects"
                })
            })
            .when(section_link, |el| {
                let id = if in_threads {
                    "threads-breadcrumb"
                } else {
                    "objects-breadcrumb"
                };
                el.child(
                    div().debug_selector(move || id.into()).child(
                        Button::new(id)
                            .text()
                            .small()
                            .label(if in_threads {
                                "Threads"
                            } else {
                                "Kernel objects"
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if in_threads {
                                    this.show_overview(window, cx);
                                } else {
                                    this.follow_object_breadcrumb(None, window, cx);
                                }
                            })),
                    ),
                )
            })
            .when_some(kind, |el, kind| {
                el.child("›").child(
                    div()
                        .debug_selector(|| "object-type-breadcrumb".into())
                        .when(!is_detail, |el| el.child(kind.label()))
                        .when(is_detail, |el| {
                            el.child(
                                Button::new("object-type-breadcrumb")
                                    .text()
                                    .small()
                                    .label(kind.label())
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.follow_object_breadcrumb(Some(kind), window, cx);
                                    })),
                            )
                        }),
                )
            })
            .when_some(self.screen.name(), |el, name| {
                el.child("›").child(
                    div()
                        .debug_selector(|| "breadcrumb-current".into())
                        .text_color(rgb(0xe6dfef))
                        .child(name.to_string()),
                )
            })
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let objects = kernel_model::rows(&self.frames[self.frame]);
        let in_objects = matches!(
            self.screen,
            Screen::KernelObjects
                | Screen::Semaphore(_)
                | Screen::MessageQueue(_)
                | Screen::MemorySlab(_)
                | Screen::Mutex(_)
                | Screen::WorkQueue(_)
                | Screen::Heap(_)
        );
        let types = std::iter::once(None)
            .chain(kernel_model::Kind::ALL.map(Some))
            .map(|kind| {
                let label = kind.map_or("All types", kernel_model::Kind::label);
                let count = objects
                    .iter()
                    .filter(|row| kind.is_none_or(|kind| row.kind == kind))
                    .count();
                let selector = kind.map_or_else(
                    || "object-type-all".to_string(),
                    |kind| format!("object-type-{}", kind.label()),
                );
                SidebarMenuItem::new(label)
                    .active(in_objects && self.object_kind == kind)
                    .suffix(move |_, _| {
                        div()
                            .debug_selector({
                                let selector = selector.clone();
                                move || selector.clone()
                            })
                            .text_xs()
                            .child(count.to_string())
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.show_kernel_objects(kind, window, cx);
                    }))
            })
            .collect::<Vec<_>>();
        Sidebar::new("navigation").w(px(230.)).collapsed(self.collapsed).collapsible(true)
            .header(SidebarHeader::new().p_4().child(
                div().flex().items_center().gap_3()
                    .child(div().size_9().rounded_lg().bg(rgb(0x7561cd)).flex().items_center().justify_center().child("Z"))
                    .when(!self.collapsed, |el| el.child(div().flex().flex_col()
                        .child(div().font_semibold().child("ZView"))
                        .child(div().text_xs().text_color(rgb(MUTED)).child("Zephyr runtime inspector"))))
            ))
            .child(SidebarGroup::new("Monitor").child(SidebarMenu::new()
                .child(SidebarMenuItem::new("Threads").icon(Icon::new(assets::IconName::Spool)).active(!in_objects)
                    .on_click(cx.listener(|this, _, window, cx| this.show_overview(window, cx))))
                .child(SidebarMenuItem::new("Kernel objects").icon(Icon::new(assets::IconName::Boxes))
                        .active(in_objects)
                        .default_open(in_objects)
                        .click_to_open(true)
                        .children(types)
                        .suffix({
                            let count = objects.len();
                            move |_, _| div().debug_selector(|| "kernel-navigation".into()).text_xs().child(count.to_string())
                        })
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.show_kernel_objects(this.object_kind, window, cx);
                        })))))
            .child(SidebarGroup::new("Session").child(SidebarMenu::new()
                .child(SidebarMenuItem::new("Help & shortcuts").icon(IconName::Info)
                    .on_click(cx.listener(|_, _, window, cx| {
                        window.open_dialog(cx, |dialog, _, _| dialog.title("ZView desktop").child(
                            concat!(
                                "Open dump loads zview dump --json output. Open recording asks for a .ndjson.gz recording and its matching ELF; Python with ZView dependencies is required (ZVIEW_PYTHON). ",
                                "Reload repeats the import. Cancel or a failed import leaves the current session intact. Arrays enable frame navigation and charts. ",
                                "Click a thread row to open details. Focus the thread table and use Up/Down then Enter. ",
                                "Escape or Back to Threads returns without changing session time. CPU excludes idle. ",
                                "Stack watermark is not total device RAM. Options are raw target flags. ",
                                "Expand Kernel objects in the sidebar to filter by type; search filters by name/address. ",
                                "Click a kernel-object row, or focus it with Tab and press Enter, for details. ",
                                "Escape and Back return with filters intact. Breadcrumbs open their named list with search cleared; Kernel objects shows all types. ",
                                "Previous/Next changes the shared snapshot. Histories show sampled counts/depth, not individual operations. ",
                                "State bars show counts, occupancy, or lock/busy state—not CPU usage. ",
                                "Unknown wait queues are labelled unavailable; known empty queues are blank. ",
                                "Heap maps show captured chunks and relative offsets, not absolute block addresses."
                            )
                        ));
                    })))))
            .footer(SidebarFooter::new().p_4().when(!self.collapsed, |el| el.child(
                div().flex().flex_col().gap_2().text_xs().text_color(rgb(MUTED))
                    .child(if self.recording_elf.is_some() { "RECORDING SESSION" } else if self.source.is_some() { "DUMP SESSION" } else { "BUNDLED SAMPLE" })
                    .child(format!("Frame {} of {} · offline", self.frame + 1, self.frames.len()))
            )))
    }

    fn chart_card(
        id: &'static str,
        title: &'static str,
        value: Option<f64>,
        detail: String,
        color: u32,
        series: model::ChartSeries,
        selector: Option<AnyElement>,
    ) -> impl IntoElement + Styled {
        div()
            .debug_selector(move || id.into())
            .flex_1()
            .min_w_0()
            .min_h_0()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(CARD))
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .debug_selector(move || format!("{id}-header"))
                    .flex()
                    .min_h(px(28.))
                    .flex_shrink_0()
                    .items_start()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .debug_selector(move || format!("{id}-title"))
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child(title),
                    )
                    .when_some(selector, |el, selector| el.child(selector)),
            )
            .child(
                div()
                    .debug_selector(move || format!("{id}-value"))
                    .text_2xl()
                    .font_semibold()
                    .text_color(rgb(color))
                    .child(percent(value)),
            )
            .child(layout::note(format!("{id}-description"), detail))
            .child(
                div()
                    .debug_selector(move || format!("{id}-plot"))
                    .flex_1()
                    .min_h_0()
                    .child(time_series_chart::TimeSeriesChart::new(
                        id,
                        series,
                        rgb(color).into(),
                    )),
            )
    }

    fn table(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let frame = &self.frames[self.frame];
        let rows = frame.visible_threads();
        div()
            .id("thread-table")
            .debug_selector(|| "thread-table".into())
            .track_focus(&self.table_focus)
            .tab_group()
            .key_context("ThreadTable")
            .on_action(cx.listener(Self::open_selected))
            .on_action(
                cx.listener(|this, _: &SelectNextThread, _, cx| this.select_thread(true, cx)),
            )
            .on_action(
                cx.listener(|this, _: &SelectPreviousThread, _, cx| this.select_thread(false, cx)),
            )
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .id("thread-columns")
                    .debug_selector(|| "thread-table-viewport".into())
                    .relative()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .overflow_x_scroll()
                    .lock_scroll_axis()
                    .track_scroll(&self.thread_columns_scroll)
                    .horizontal_scrollbar(&self.thread_columns_scroll)
                    .child(
                        div()
                            .w_full()
                            .h_full()
                            .flex()
                            .flex_col()
                            .flex_shrink_0()
                            .min_w(px(900.))
                            .rounded_lg()
                            .border_1()
                            .border_color(rgb(BORDER))
                            .overflow_hidden()
                            .child(
                                div()
                                    .debug_selector(|| "thread-table-header".into())
                                    .flex()
                                    .flex_shrink_0()
                                    .p_3()
                                    .bg(rgb(0x2a263e))
                                    .text_xs()
                                    .text_color(rgb(MUTED))
                                    .child(div().flex_1().min_w(px(180.)).child("THREAD"))
                                    .child(div().w(px(95.)).child("CPU %"))
                                    .child(div().w(px(95.)).child("LOAD %"))
                                    .child(div().w(px(250.)).child("STACK WATERMARK"))
                                    .child(div().w(px(160.)).child("WATERMARK / SIZE")),
                            )
                            .child(
                                div()
                                    .id("thread-rows")
                                    .debug_selector(|| "thread-rows".into())
                                    .relative()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .lock_scroll_axis()
                                    .track_scroll(&self.thread_rows_scroll)
                                    .vertical_scrollbar(&self.thread_rows_scroll)
                                    .children(rows.iter().enumerate().map(|(i, thread)| {
                                        let address = thread.address;
                                        let runtime = thread.runtime.as_ref();
                                        let active = runtime.is_some_and(|r| r.active);
                                        let watermark =
                                            runtime.and_then(|r| r.stack_watermark_percent);
                                        let color = if watermark.is_some_and(|v| v >= 70.) {
                                            0xddb66b
                                        } else {
                                            GREEN
                                        };
                                        div()
                                            .id(("thread", i))
                                            .debug_selector(move || format!("thread-row-{address}"))
                                            .flex()
                                            .flex_shrink_0()
                                            .items_center()
                                            .p_3()
                                            .text_sm()
                                            .bg(rgb(if self.selected == Some(address) {
                                                0x3b3053
                                            } else if i % 2 == 0 {
                                                0x242137
                                            } else {
                                                BG
                                            }))
                                            .cursor_pointer()
                                            .hover(|el| el.bg(rgb(0x353049)))
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.selected = Some(address);
                                                this.open_selected(&OpenThread, window, cx);
                                            }))
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w(px(180.))
                                                    .flex()
                                                    .gap_3()
                                                    .items_center()
                                                    .child(
                                                        div()
                                                            .id(("thread-activity", i))
                                                            .debug_selector(move || {
                                                                format!(
                                                                    "thread-activity-{}-{address}",
                                                                    if active {
                                                                        "active"
                                                                    } else {
                                                                        "inactive"
                                                                    }
                                                                )
                                                            })
                                                            .tooltip({
                                                                let description =
                                                                    thread.activity_description();
                                                                move |window, cx| {
                                                                    Tooltip::new(description)
                                                                        .build(window, cx)
                                                                }
                                                            })
                                                            .size_4()
                                                            .flex_shrink_0()
                                                            .child(
                                                                Icon::new(if active {
                                                                    assets::IconName::Activity
                                                                } else {
                                                                    assets::IconName::CircleDashed
                                                                })
                                                                .size_4()
                                                                .text_color(rgb(if active {
                                                                    GREEN
                                                                } else {
                                                                    0x686078
                                                                })),
                                                            ),
                                                    )
                                                    .child(thread.name.clone()),
                                            )
                                            .child(div().w(px(95.)).child(number(
                                                runtime.and_then(|r| r.cpu_normalized),
                                            )))
                                            .child(
                                                div()
                                                    .w(px(95.))
                                                    .child(number(runtime.and_then(|r| r.cpu))),
                                            )
                                            .child(
                                                div()
                                                    .w(px(250.))
                                                    .flex()
                                                    .items_center()
                                                    .gap_3()
                                                    .child(div().w(px(160.)).when_some(
                                                        watermark,
                                                        |el, v| {
                                                            el.child(
                                                                Progress::new(("stack", i))
                                                                    .value(v as f32)
                                                                    .color(rgb(color))
                                                                    .small()
                                                                    .accessibility_label(format!(
                                                                        "{} stack watermark",
                                                                        thread.name
                                                                    )),
                                                            )
                                                        },
                                                    ))
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(rgb(color))
                                                            .child(percent(watermark)),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .w(px(160.))
                                                    .text_xs()
                                                    .text_color(rgb(MUTED))
                                                    .child(format!(
                                                        "{} / {} B",
                                                        bytes(
                                                            runtime.and_then(|r| r.stack_watermark)
                                                        ),
                                                        bytes(thread.stack_size)
                                                    )),
                                            )
                                    }))
                                    .when(rows.is_empty(), |el| {
                                        el.child(
                                            div()
                                                .p_8()
                                                .text_color(rgb(MUTED))
                                                .child("No non-idle threads in this snapshot."),
                                        )
                                    }),
                            ),
                    ),
            )
    }
}

fn number(value: Option<f64>) -> String {
    value
        .map(|v| format!("{v:.2}"))
        .unwrap_or_else(|| "—".into())
}

fn bytes(value: Option<u64>) -> String {
    value.map(|v| v.to_string()).unwrap_or_else(|| "—".into())
}

impl Render for ThreadsApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.playing {
            // A display frame can arrive before the source timer callback.
            // Keep the discrete reading aligned with this frame's clock too.
            if let Some(time) = self.playback_time(cx)
                && model::frame_at(&self.frames, time) != self.frame
            {
                self.playback_tick(time, window, cx);
            }
        }
        if self.playing {
            let app = cx.entity().downgrade();
            window.on_next_frame(move |_, cx| {
                let _ = app.update(cx, |this, cx| {
                    if this.playing {
                        cx.notify();
                    }
                });
            });
            window.request_animation_frame();
        }
        self.sync_missing_notice(window, cx);
        let frame = &self.frames[self.frame];
        let selected = self.heap.read(cx).selected_value();
        let heap = frame.heaps.iter().find(|h| Some(&h.label()) == selected);
        let heap_key = heap.map(|h| model::HeapKey {
            address: h.address,
            name: h.name.clone(),
        });
        let stack = frame.stacks();
        // Reserve the widest status for this document, independent of the
        // current digits or Playing/Paused state. The hidden template uses the
        // same font as the visible text, so this also follows theme font sizes.
        let status_template = self
            .frames
            .last()
            .and_then(|f| f.elapsed_seconds)
            .map(|end| format!("Playing · {end:.2}s / {end:.2}s · 1×"))
            .unwrap_or_else(|| format!("{} / {}", self.frames.len(), self.frames.len()));
        div()
            .size_full()
            .flex()
            .bg(rgb(BG))
            .text_color(rgb(0xe6dfef))
            .when(matches!(self.screen, Screen::Thread(_)), |el| {
                el.track_focus(&self.detail_focus)
                    .key_context("ThreadDetail")
                    .on_action(cx.listener(|this, _: &BackToThreads, window, cx| {
                        this.show_overview(window, cx)
                    }))
            })
            .when(matches!(self.screen, Screen::Semaphore(_) | Screen::MessageQueue(_) | Screen::MemorySlab(_) | Screen::Mutex(_) | Screen::WorkQueue(_) | Screen::Heap(_)), |el| {
                el.track_focus(&self.detail_focus)
                    .key_context("SemaphoreDetail")
                    .on_action(cx.listener(|this, _: &BackToObjects, window, cx| {
                        this.show_kernel_objects(this.object_kind, window, cx)
                    }))
            })
            .child(self.sidebar(cx))
            .child(
                div()
                    .id(match self.screen {
                        Screen::Overview => "overview-content",
                        Screen::Thread(_) => "detail-content",
                        Screen::KernelObjects => "kernel-content",
                        Screen::Semaphore(_) => "semaphore-content",
                        Screen::MessageQueue(_) => "msgq-content",
                        Screen::MemorySlab(_) => "slab-content",
                        Screen::Mutex(_) => "mutex-content",
                        Screen::WorkQueue(_) => "workq-content",
                        Screen::Heap(_) => "heap-content",
                    })
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .p_4()
                    .gap_3()
                    .overflow_hidden()
                    .child(
                        div()
                            .debug_selector(|| "app-toolbar".into())
                            .flex()
                            .flex_shrink_0()
                            .flex_wrap()
                            .gap_3()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_4()
                                    .items_center()
                                    .child(
                                        SidebarToggleButton::new()
                                            .collapsed(self.collapsed)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.collapsed = !this.collapsed;
                                                cx.notify();
                                            })),
                                    )
                                    .child(self.breadcrumbs(cx)),
                            )
                            .child(self.session_menu(cx)),
                    )
                    .when(self.screen == Screen::Overview, |el| {
                        el.child(
                            div()
                                .flex()
                                .flex_shrink_0()
                                .flex_col()
                                .gap_1()
                                .child(div().text_2xl().font_semibold().child("Threads"))
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(rgb(MUTED))
                                        .child("Runtime activity and thread stack usage"),
                                ),
                        )
                    })
                    .when_some(self.error.clone(), |el, error| {
                        el.child(div().p_3().flex_shrink_0().rounded_md().bg(rgb(0x492c3c))
                            .child(layout::note("session-error", error)))
                    })
                    .when(self.loading.is_some(), |el| {
                        el.child(
                            div().debug_selector(|| "session-loading".into())
                                .flex().flex_shrink_0().items_center().gap_3().p_3().rounded_md().bg(rgb(CARD))
                                .child("Loading session… Current session stays available.")
                                .child(div().debug_selector(|| "cancel-load".into()).child(Button::new("cancel-load").label("Cancel")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(cancel) = &this.loading {
                                            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        cx.notify();
                                    })))),
                        )
                    })
                    .when_some(
                        match &self.screen {
                            Screen::Thread(key) => Some(key),
                            _ => None,
                        },
                        |el, key| el.child(self.thread_detail(key, cx)),
                    )
                    .when(self.screen == Screen::KernelObjects, |el| {
                        el.child(self.kernel_objects(cx))
                    })
                    .when_some(
                        match &self.screen {
                            Screen::Semaphore(key) => Some(key),
                            _ => None,
                        },
                        |el, key| el.child(self.semaphore_detail(key, cx)),
                    )
                    .when_some(
                        match &self.screen {
                            Screen::MessageQueue(key) => Some(key),
                            _ => None,
                        },
                        |el, key| el.child(self.msgq_detail(key, cx)),
                    )
                    .when_some(
                        match &self.screen {
                            Screen::MemorySlab(key) => Some(key),
                            _ => None,
                        },
                        |el, key| el.child(self.slab_detail(key, cx)),
                    )
                    .when_some(
                        match &self.screen {
                            Screen::Mutex(key) => Some(key),
                            _ => None,
                        },
                        |el, key| el.child(self.mutex_detail(key, cx)),
                    )
                    .when_some(
                        match &self.screen {
                            Screen::WorkQueue(key) => Some(key),
                            _ => None,
                        },
                        |el, key| el.child(self.workq_detail(key, window, cx)),
                    )
                    .when_some(
                        match &self.screen {
                            Screen::Heap(key) => Some(key),
                            _ => None,
                        },
                        |el, key| el.child(self.heap_detail(key, window, cx)),
                    )
                    .when(self.screen == Screen::Overview, |el| {
                        el.child(
                            div()
                                .debug_selector(|| "overview-charts".into())
                                .flex()
                                .flex_1()
                                .min_h_0()
                                .gap_3()
                                .child(
                                    Self::chart_card(
                                        "cpu",
                                        "CPU USAGE",
                                        frame.cpu(),
                                        "Non-idle CPU time".into(),
                                        PURPLE,
                                        model::chart_series(&self.frames, self.frame, Frame::cpu)
                                            .with_playhead(self.playback_time(cx)),
                                        None,
                                    )
                                    .h_auto(),
                                )
                                .child(
                                    Self::chart_card(
                                        "stack",
                                        "STACK WATERMARK",
                                        stack.and_then(|(u, s)| percentage(u, s)),
                                        stack
                                            .map(|(u, s)| {
                                                format!("{u} / {s} B · not total device RAM")
                                            })
                                            .unwrap_or_else(|| {
                                                "Stack measurements unavailable".into()
                                            }),
                                        GREEN,
                                        model::chart_series(&self.frames, self.frame, |f| {
                                            f.stacks().and_then(|(u, s)| percentage(u, s))
                                        }).with_playhead(self.playback_time(cx)),
                                        None,
                                    )
                                    .h_auto(),
                                )
                                .child(
                                    Self::chart_card(
                                        "heap",
                                        "HEAP USAGE",
                                        heap.and_then(|h| h.percent()),
                                        heap.map(|h| {
                                            format!("{} / {} B", h.allocated_bytes, h.total())
                                        })
                                        .unwrap_or_else(|| "No heaps available".into()),
                                        BLUE,
                                        model::heap_usage_series(&self.frames, self.frame, heap_key.as_ref())
                                            .with_playhead(self.playback_time(cx)),
                                        Some(
                                            div().debug_selector(|| "heap-selector".into())
                                                .flex_1().min_w_0().max_w(px(140.)).child(
                                            Select::new(&self.heap)
                                                .small()
                                                .w_full()
                                                .disabled(frame.heaps.is_empty())
                                                .placeholder("No heaps available")
                                                .accessibility_label("Heap"))
                                                .into_any_element(),
                                        ),
                                    )
                                    .h_auto(),
                                ),
                        )
                        .child(div().debug_selector(|| "thread-table-heading".into())
                            .flex().flex_shrink_0().justify_between()
                            .child(format!("All threads · {}", frame.visible_threads().len()))
                            .child(div().text_xs().text_color(rgb(MUTED))
                                .child("Click to inspect · ↑/↓ select · Enter to inspect")))
                        .child(self.table(cx))
                        .child(layout::note("thread-activity-legend",
                            "Green activity icon: CPU activity since previous sample · Gray dashed circle: no observed activity or unavailable · Not current scheduler state"))
                    })
                    .child(
                        div()
                            .debug_selector(|| "session-footer".into())
                            .flex()
                            .min_w_0()
                            .gap_3()
                            .flex_shrink_0()
                            .items_center()
                            .justify_between()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child(div().flex_1().min_w_0().child(layout::note("session-source",
                                self.source
                                    .as_ref()
                                    .map(|p| match &self.recording_elf {
                                        Some(elf) => format!("{} · ELF: {}", p.display(), elf.display()),
                                        None => p.display().to_string(),
                                    })
                                    .unwrap_or_else(|| {
                                        "Bundled real dump · sys_heap_v4.3 recording, frame 2"
                                            .into()
                                    }),
                            )))
                            .child(
                                div()
                                    .flex()
                                    .flex_shrink_0()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        div().debug_selector(|| "previous".into()).child(Button::new("previous")
                                            .small()
                                            .label("Previous")
                                            .disabled(self.frame == 0)
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.step(false, window, cx)
                                            }))),
                                    )
                                    .child(div().debug_selector(|| "play-pause".into()).child(
                                        Button::new("play-pause").small()
                                            .icon(if self.playing { IconName::Pause } else { IconName::Play })
                                            .tooltip(if self.playing { "Pause" } else { "Play" })
                                            .accessibility_label(if self.playing { "Pause playback" } else { "Play recording" })
                                            .disabled(self.frames.len() < 2 || frame.elapsed_seconds.is_none())
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.toggle_playback(window, cx);
                                            }))))
                                    .child(div().debug_selector(|| "reset-playback".into()).child(
                                        Button::new("reset-playback").small().icon(IconName::RotateCw)
                                            .tooltip("Reset to beginning")
                                            .accessibility_label("Reset playback to beginning")
                                            .disabled(self.frame == 0 && !self.playing)
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.reset_playback(window, cx);
                                            }))))
                                    .child(
                                        div().debug_selector(|| "playback-status".into())
                                            .relative().flex_shrink_0()
                                            .font_family(cx.theme().mono_font_family.clone())
                                            .child(div().invisible().child(status_template))
                                            .child(div().absolute().top_0().left_0().child(
                                        self.playback_time(cx).or(frame.elapsed_seconds)
                                            .map(|t| {
                                                let end = self.frames.last().and_then(|f| f.elapsed_seconds).unwrap();
                                                let width = format!("{end:.2}").len();
                                                format!(
                                                    "{:<7} · {t:>width$.2}s / {end:.2}s · 1×",
                                                    if self.playing { "Playing" } else { "Paused" },
                                                )
                                            })
                                            .unwrap_or_else(|| {
                                                format!(
                                                    "{} / {}",
                                                    self.frame + 1,
                                                    self.frames.len()
                                                )
                                            }))),
                                    )
                                    .child(
                                        div().debug_selector(|| "next".into()).child(Button::new("next")
                                            .small()
                                            .label("Next")
                                            .disabled(self.frame + 1 == self.frames.len())
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.step(true, window, cx)
                                            }))),
                                    ),
                            ),
                    ),
            )
    }
}

fn apply_theme(cx: &mut App) {
    Theme::change(ThemeMode::Dark, None, cx);
    Theme::update(cx, |theme| {
        theme.colors.background = rgb(BG).into();
        theme.colors.foreground = rgb(0xe6dfef).into();
        theme.colors.muted = rgb(CARD).into();
        theme.colors.muted_foreground = rgb(MUTED).into();
        theme.colors.border = rgb(BORDER).into();
        theme.colors.input = rgb(BORDER).into();
        theme.colors.ring = rgb(PURPLE).into();
        theme.colors.button = rgb(CARD).into();
        theme.colors.button_foreground = rgb(0xe6dfef).into();
        theme.colors.button_hover = rgb(0x353049).into();
        theme.colors.button_active = rgb(0x3d365b).into();
        theme.colors.secondary = rgb(CARD).into();
        theme.colors.secondary_foreground = rgb(0xe6dfef).into();
        theme.colors.secondary_hover = rgb(0x353049).into();
        theme.colors.secondary_active = rgb(0x3d365b).into();
        theme.colors.button_secondary = rgb(CARD).into();
        theme.colors.button_secondary_foreground = rgb(0xe6dfef).into();
        theme.colors.button_secondary_hover = rgb(0x353049).into();
        theme.colors.button_secondary_active = rgb(0x3d365b).into();
        theme.colors.popover = rgb(CARD).into();
        theme.colors.popover_foreground = rgb(0xe6dfef).into();
        theme.colors.accent = rgb(0x3d365b).into();
        theme.colors.accent_foreground = rgb(0xe6dfef).into();
        theme.colors.selection = rgb(0x3d365b).into();
        theme.colors.list = rgb(CARD).into();
        theme.colors.list_hover = rgb(0x353049).into();
        theme.colors.list_active = rgb(0x3d365b).into();
        theme.colors.list_active_border = rgb(PURPLE).into();
        theme.colors.list_head = rgb(CARD).into();
        theme.colors.list_even = rgb(CARD).into();
        theme.colors.sidebar = rgb(0x23213b).into();
        theme.colors.sidebar_accent = rgb(0x3d365b).into();
        theme.colors.sidebar_border = rgb(BORDER).into();
        theme.colors.primary = rgb(PURPLE).into();
    });
}

/// Run the native application. The browser entry point lives in `web`.
#[cfg(not(target_family = "wasm"))]
pub fn run() {
    application().with_assets(app_assets::AppAssets).run(|cx| {
        init(cx);
        bind_keys(cx);
        apply_theme(cx);
        open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1480.), px(940.)),
                    cx,
                ))),
                window_min_size: Some(size(px(1000.), px(680.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("ZView".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| ThreadsApp::new(window, cx)),
        )
        .expect("open ZView window");
        cx.activate(true);
    });
}

fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("enter", OpenThread, Some("ThreadTable")),
        KeyBinding::new("down", SelectNextThread, Some("ThreadTable")),
        KeyBinding::new("up", SelectPreviousThread, Some("ThreadTable")),
        KeyBinding::new("escape", BackToThreads, Some("ThreadDetail")),
        KeyBinding::new("escape", BackToObjects, Some("SemaphoreDetail")),
        KeyBinding::new("enter", OpenSemaphore, Some("SemaphoreRow")),
        KeyBinding::new("enter", OpenMessageQueue, Some("MessageQueueRow")),
        KeyBinding::new("enter", OpenMemorySlab, Some("MemorySlabRow")),
        KeyBinding::new("enter", OpenMutex, Some("MutexRow")),
        KeyBinding::new("enter", OpenWorkQueue, Some("WorkQueueRow")),
        KeyBinding::new("enter", OpenHeap, Some("HeapRow")),
    ]);
}

//! Version-5 web adapter contract and pure client state, testable without a browser.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const PROTOCOL_VERSION: u32 = 5;

#[derive(Clone, Debug, Deserialize)]
pub struct Envelope {
    pub version: u32,
    pub session_id: Option<String>,
    pub sequence: u64,
    #[serde(flatten)]
    pub event: Event,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum Event {
    Snapshot(Option<Box<Session>>),
    Status(StatusUpdate),
    Frame(Frame),
    Notice { message: String },
    Health { samples_per_second: Option<f64> },
    Capabilities(Capabilities),
    HeapDetail(HeapDetail),
    View(Box<LiveViewState>),
    UsageWindow { polls: u32 },
}

#[derive(Clone, Debug, Deserialize)]
pub struct Session {
    pub id: String,
    pub source: Box<Source>,
    pub elf: String,
    pub status: Status,
    pub frames_received: u64,
    pub last_frame: Option<Frame>,
    pub last_notice: Option<String>,
    #[serde(default)]
    pub read_warning: bool,
    pub error: Option<String>,
    #[serde(default)]
    pub context: Option<Box<BuildContext>>,
    pub samples_per_second: Option<f64>,
    pub capabilities: Option<Capabilities>,
    #[serde(default)]
    pub heap_detail: Box<HeapDetail>,
    #[serde(default)]
    pub timing: Option<Box<AcquisitionTiming>>,
    #[serde(default)]
    pub inventory: Option<BTreeMap<String, Vec<HeapIdentity>>>,
    #[serde(default)]
    pub live_view: Option<Box<LiveViewState>>,
    #[serde(default)]
    pub usage_window: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "screen", rename_all = "snake_case")]
pub enum LiveView {
    Overview { heap: Option<HeapIdentity> },
    Thread { object: HeapIdentity },
    Objects { kind: Option<String>, query: String },
    Detail { group: String, object: HeapIdentity },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LiveViewState {
    pub revision: u64,
    pub owner: Option<String>,
    pub view: LiveView,
    #[serde(default = "default_usage_window")]
    pub usage_window: u32,
}

impl LiveViewState {
    /// Release retains the last selection, but that selection no longer blocks
    /// another tab's retry, even when it wants a different screen.
    pub fn active_view(&self) -> Option<&LiveView> {
        self.owner.as_ref().map(|_| &self.view)
    }
}

fn default_usage_window() -> u32 {
    16
}

fn validate_usage_window(polls: u32) -> Result<(), String> {
    if !(1..=64).contains(&polls) {
        return Err("CPU averaging must be from 1 to 64 polls.".into());
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AcquisitionTiming {
    pub period: f64,
    pub heap_period: f64,
    pub heap_cooldown: f64,
}

impl AcquisitionTiming {
    fn validate(&self) -> Result<(), String> {
        if !self.period.is_finite()
            || self.period < 0.
            || !self.heap_period.is_finite()
            || self.heap_period <= 0.
            || !self.heap_cooldown.is_finite()
            || self.heap_cooldown < 0.
        {
            return Err("Invalid acquisition timing.".into());
        }
        Ok(())
    }

    pub fn polling_label(&self) -> String {
        if self.period == 0. {
            "Requested polling: back-to-back (period 0 s).".into()
        } else {
            format!("Requested polling: {} s minimum period.", self.period)
        }
    }

    pub fn heap_label(&self) -> String {
        format!(
            "Requested map period: {} s · cooldown after each read: {} s.",
            self.heap_period, self.heap_cooldown
        )
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct HeapIdentity {
    pub address: u64,
    pub name: String,
}

/// A tab's HTTP interest lifecycle. Replies belong to a particular binding,
/// while same-session navigation waits for the preceding request to finish.
#[derive(Clone, Debug)]
pub struct RequestInterest<T> {
    session: Option<String>,
    attempted: Option<Option<T>>,
    pending: bool,
    epoch: u64,
    pub error: Option<String>,
    conflict: bool,
}

impl<T> Default for RequestInterest<T> {
    fn default() -> Self {
        Self {
            session: None,
            attempted: None,
            pending: false,
            epoch: 0,
            error: None,
            conflict: false,
        }
    }
}

pub type HeapInterest = RequestInterest<HeapIdentity>;
pub type ViewInterest = RequestInterest<LiveView>;

impl<T: PartialEq> RequestInterest<T> {
    pub fn reset(&mut self) {
        self.session = None;
        self.attempted = None;
        self.pending = false;
        self.error = None;
        self.conflict = false;
        self.epoch += 1;
    }

    pub fn bind(&mut self, session: Option<&str>) {
        if self.session.as_deref() != session {
            self.reset();
            self.session = session.map(str::to_owned);
        }
    }

    pub fn begin(&mut self, heap: Option<T>, active: Option<&T>) -> Option<u64> {
        if self.session.is_none() || self.pending {
            return None;
        }
        // A conflict can clear before or after its HTTP reply arrives. It also
        // clears when another tab starts the heap we want: we may share it.
        if self.conflict && (active.is_none() || active == heap.as_ref()) {
            self.attempted = None;
        }
        if self.attempted.as_ref() == Some(&heap) || (self.attempted.is_none() && heap.is_none()) {
            return None;
        }
        self.attempted = Some(heap);
        self.pending = true;
        self.error = None;
        self.conflict = false;
        self.epoch += 1;
        Some(self.epoch)
    }

    pub fn finish(&mut self, epoch: u64, error: Option<String>, conflict: bool) {
        if self.epoch == epoch {
            self.pending = false;
            self.error = error;
            self.conflict = conflict;
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct HeapDetail {
    pub heap: Option<HeapIdentity>,
    pub last_map: Option<CapturedHeapMap>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CapturedHeapMap {
    pub heap: HeapIdentity,
    pub chunks: Vec<crate::model::HeapChunk>,
    pub captured_at: f64,
    pub live_elapsed: f64,
    pub frame_number: u64,
}

impl HeapDetail {
    fn validate(&self) -> Result<(), String> {
        if self
            .heap
            .as_ref()
            .is_some_and(|h| h.address == 0 || h.name.is_empty())
        {
            return Err("Invalid detailed heap identity.".into());
        }
        if let Some(map) = &self.last_map
            && (map.heap.address == 0
                || map.heap.name.is_empty()
                || !map.captured_at.is_finite()
                || !map.live_elapsed.is_finite()
                || map.live_elapsed < 0.
                || map.frame_number == 0
                || self.heap.as_ref().is_some_and(|heap| *heap != map.heap))
        {
            return Err("Invalid captured heap map.".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Capabilities {
    pub groups: Vec<String>,
    pub missing: Vec<String>,
    pub is_waitq_walkable: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum Source {
    Live {
        configuration: LiveConfiguration,
        runner: Option<String>,
        target: Option<String>,
        frequency: Option<u64>,
    },
    Replay {
        recording: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LiveConfiguration {
    pub build_dir: String,
    pub domain: Option<String>,
    pub elf: Option<String>,
    pub runner: Option<String>,
    pub target: Option<String>,
    pub frequency: Option<u64>,
}

impl Session {
    pub fn mode(&self) -> Mode {
        match self.source.as_ref() {
            Source::Live { .. } => Mode::Live,
            Source::Replay { .. } => Mode::Replay,
        }
    }

    pub fn recording(&self) -> Option<&str> {
        match self.source.as_ref() {
            Source::Replay { recording } => Some(recording),
            Source::Live { .. } => None,
        }
    }

    pub fn source_label(&self) -> String {
        match self.source.as_ref() {
            Source::Replay { recording } => recording.clone(),
            Source::Live { .. } => format!("Live · {}", self.elf),
        }
    }

    pub fn status_label(&self) -> &str {
        if self.mode() == Mode::Replay {
            return self.status.as_str();
        }
        match self.status {
            Status::Opening => "Connecting",
            Status::Reading => "Polling",
            Status::Stopping => "Disconnecting",
            Status::Stopped => "Disconnected",
            Status::Lost => "Connection lost",
            Status::Failed => "Connection failed",
            Status::Ended => "Disconnected",
        }
    }
}

/// Resolved by Python without opening a probe. Paths are relative to its data root.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BuildContext {
    pub build_dir: String,
    pub image_dir: String,
    pub domain: String,
    pub domains: Vec<String>,
    pub elf: String,
    #[serde(default)]
    pub elf_override: Option<String>,
    pub board: Option<String>,
    pub project: String,
    pub source_dir: Option<String>,
    pub git: Option<GitContext>,
    pub runner: Option<String>,
    pub target: Option<String>,
    pub frequency: Option<u64>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GitContext {
    pub commit: String,
    pub branch: Option<String>,
    pub dirty: bool,
    pub provenance: String,
}

impl BuildContext {
    pub fn tooltip(&self) -> String {
        let mut lines = vec![
            format!("Project: {}", self.project),
            format!("Build directory: {}", self.build_dir),
            format!("Firmware image: {}", self.domain),
            format!("ELF: {}", self.elf),
            format!("Board: {}", self.board.as_deref().unwrap_or("Unknown")),
            "Context resolved on this adapter, not provenance embedded in the recording.".into(),
        ];
        if let Some(source) = &self.source_dir {
            lines.push(format!("Source directory: {source}"));
        }
        if let Some(git) = &self.git {
            lines.push(format!(
                "Checkout observed at resolution: {} · {}{}",
                git.branch.as_deref().unwrap_or("detached HEAD"),
                git.commit,
                if git.dirty { " · tracked changes" } else { "" }
            ));
            lines.push("Checkout metadata is not proof of the firmware's build revision.".into());
        } else {
            lines.push("Git provenance unavailable.".into());
        }
        lines.join("\n")
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Live,
    Replay,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PickerKind {
    Directory,
    Elf,
    Recording,
}

impl PickerKind {
    pub fn title(self) -> &'static str {
        match self {
            Self::Directory => "Choose build directory",
            Self::Elf => "Choose matching ELF",
            Self::Recording => "Choose recording",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FileChoice {
    /// Null is a normal cancellation, not an empty path or an acquisition error.
    pub path: Option<String>,
}

/// Compact, truthful session labels; draft configuration is never an active source.
pub fn session_labels(session: Option<&Session>, connection: Connection) -> [String; 4] {
    let Some(session) = session else {
        return [
            "OPEN SESSION".into(),
            "Connect or open a recording".into(),
            connection.label().into(),
            "Configure session…".into(),
        ];
    };
    let board = session
        .context
        .as_ref()
        .and_then(|c| c.board.as_deref())
        .unwrap_or("Board unknown");
    let source = match session.source.as_ref() {
        Source::Replay { recording } => recording
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(recording)
            .to_owned(),
        Source::Live { runner, .. } => match runner.as_deref() {
            Some("jlink") => "J-Link".into(),
            Some("pyocd") => "PyOCD".into(),
            Some("gdb") => "GDB".into(),
            Some(runner) => runner.into(),
            None => "Default runner".into(),
        },
    };
    let status = if connection == Connection::Connected {
        if session.mode() == Mode::Live {
            if session.status == Status::Reading {
                session
                    .samples_per_second
                    .filter(|v| v.is_finite() && *v >= 0.)
                    .map(|v| format!("{v:.1} samples/s"))
                    .unwrap_or_else(|| "— samples/s".into())
            } else {
                "Not sampling".into()
            }
        } else {
            format!("{} decoded frames", session.frames_received)
        }
    } else {
        connection.label().into()
    };
    [
        format!(
            "{} · {}",
            if session.mode() == Mode::Live {
                "LIVE"
            } else {
                "REPLAY"
            },
            session.status_label()
        ),
        format!("{board} · {source}"),
        status,
        session
            .context
            .as_ref()
            .map(|c| format!("{} / {}", c.project, c.build_dir))
            .unwrap_or_else(|| "Build context unavailable".into()),
    ]
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Opening,
    Reading,
    Stopping,
    Stopped,
    Ended,
    Failed,
    Lost,
}

impl Status {
    pub fn is_active(self) -> bool {
        matches!(self, Self::Opening | Self::Reading | Self::Stopping)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Opening => "opening",
            Self::Reading => "reading",
            Self::Stopping => "stopping",
            Self::Stopped => "stopped",
            Self::Ended => "ended",
            Self::Failed => "failed",
            Self::Lost => "lost",
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct StatusUpdate {
    pub status: Status,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Frame {
    /// Core acquisition time, not the recording's original timestamp.
    pub decoded_at: f64,
    /// Monotonic elapsed receipt time for Live; never an original replay clock.
    pub live_elapsed: Option<f64>,
    #[serde(default)]
    pub view_revision: u64,
    #[serde(default)]
    pub view: Option<LiveView>,
    #[serde(default)]
    pub sampled_groups: Vec<String>,
    pub groups: Groups,
}

impl Frame {
    fn validate(&self, mode: Mode) -> Result<(), String> {
        if !self.decoded_at.is_finite() {
            return Err("Invalid acquisition timestamp.".into());
        }
        match mode {
            Mode::Live if !self.live_elapsed.is_some_and(|t| t.is_finite() && t >= 0.) => {
                Err("Live frame is missing its elapsed receipt time.".into())
            }
            Mode::Replay if self.live_elapsed.is_some() => {
                Err("Replay frames cannot claim live receipt time.".into())
            }
            _ => Ok(()),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Groups {
    #[serde(default)]
    pub threads: Vec<Thread>,
    // Preserve groups not displayed in the proof table, without inventing a
    // second model for every kind already serialized by the Python package.
    #[serde(flatten)]
    pub other: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Thread {
    pub address: u64,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_idle: Option<bool>,
    pub runtime: Option<Runtime>,
    #[serde(flatten)]
    pub other: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Runtime {
    pub cpu_normalized: Option<f64>,
    pub stack_watermark_percent: Option<f64>,
    #[serde(flatten)]
    pub other: BTreeMap<String, Value>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Connection {
    #[default]
    Connecting,
    Connected,
    Retrying,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionIndicator {
    Idle,
    Busy,
    Healthy,
    Waiting,
    Warning,
    Failed,
}

impl SessionIndicator {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Busy => "busy",
            Self::Healthy => "healthy",
            Self::Waiting => "waiting",
            Self::Warning => "warning",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionAction {
    Connect,
    OpenReplay,
    Disconnect,
    StopReplay,
    Reconnect,
}

impl SessionAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Connect => "Connect",
            Self::OpenReplay => "Open replay",
            Self::Disconnect => "Disconnect",
            Self::StopReplay => "Stop decoding",
            Self::Reconnect => "Reconnect",
        }
    }
}

impl Connection {
    pub fn label(self) -> &'static str {
        match self {
            Self::Connecting => "Connecting to adapter…",
            Self::Connected => "Adapter connected",
            Self::Retrying => "Adapter disconnected · retrying…",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ClientState {
    pub session: Option<Session>,
    pub connection: Connection,
    pub pending: bool,
    pub error: Option<String>,
    sequence: Option<u64>,
}

impl ClientState {
    pub fn indicator(&self, fresh_frame: bool) -> SessionIndicator {
        if self.connection == Connection::Connecting {
            return SessionIndicator::Busy;
        }
        if self.connection != Connection::Connected {
            return SessionIndicator::Idle;
        }
        if self.pending {
            return SessionIndicator::Busy;
        }
        let Some(session) = &self.session else {
            return SessionIndicator::Idle;
        };
        match session.status {
            Status::Opening | Status::Stopping => SessionIndicator::Busy,
            Status::Failed | Status::Lost => SessionIndicator::Failed,
            Status::Ended | Status::Stopped => SessionIndicator::Idle,
            Status::Reading if session.read_warning => SessionIndicator::Warning,
            Status::Reading if !fresh_frame => SessionIndicator::Waiting,
            Status::Reading => SessionIndicator::Healthy,
        }
    }

    pub fn primary_action(&self) -> SessionAction {
        let Some(session) = &self.session else {
            return SessionAction::Connect;
        };
        match (session.mode(), session.status.is_active()) {
            (Mode::Live, true) => SessionAction::Disconnect,
            (Mode::Live, false) => SessionAction::Reconnect,
            (Mode::Replay, true) => SessionAction::StopReplay,
            (Mode::Replay, false) => SessionAction::OpenReplay,
        }
    }

    pub fn can_primary_action(&self) -> bool {
        match self.primary_action() {
            SessionAction::Connect | SessionAction::OpenReplay => self.can_open(),
            SessionAction::Disconnect | SessionAction::StopReplay => self.can_stop(),
            SessionAction::Reconnect => self.can_reconnect(),
        }
    }

    pub fn can_open(&self) -> bool {
        self.connection == Connection::Connected
            && !self.pending
            && !self.session.as_ref().is_some_and(|s| s.status.is_active())
    }

    pub fn can_stop(&self) -> bool {
        self.connection == Connection::Connected
            && !self.pending
            && self
                .session
                .as_ref()
                .is_some_and(|s| matches!(s.status, Status::Opening | Status::Reading))
    }

    pub fn can_reconnect(&self) -> bool {
        self.can_open()
            && self
                .session
                .as_ref()
                .is_some_and(|s| s.mode() == Mode::Live)
    }

    pub fn is_live(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(|s| s.mode() == Mode::Live)
    }

    /// Apply stream messages only. HTTP command replies are acknowledgements,
    /// not a competing state source. False means a stale event was ignored.
    pub fn accept(&mut self, message: Envelope) -> Result<bool, String> {
        if message.version != PROTOCOL_VERSION {
            return Err(format!(
                "Unsupported adapter protocol {} (expected {}).",
                message.version, PROTOCOL_VERSION
            ));
        }
        if let Event::Snapshot(snapshot) = message.event {
            if let Some(session) = &snapshot {
                if let Some(polls) = session.usage_window {
                    validate_usage_window(polls)?;
                }
                if let Some(view) = &session.live_view {
                    validate_usage_window(view.usage_window)?;
                }
                if let Some(timing) = &session.timing {
                    timing.validate()?;
                }
                session.heap_detail.validate()?;
                if session.mode() == Mode::Replay
                    && (session.heap_detail.heap.is_some()
                        || session.heap_detail.last_map.is_some())
                {
                    return Err("Replay cannot contain live heap capture.".into());
                }
                if let Some(frame) = &session.last_frame {
                    frame.validate(session.mode())?;
                }
                if session
                    .samples_per_second
                    .is_some_and(|v| !v.is_finite() || v < 0.)
                {
                    return Err("Invalid observed sample rate.".into());
                }
            }
            if snapshot.as_ref().map(|s| &s.id) != message.session_id.as_ref() {
                return Err("Snapshot session ID does not match its envelope.".into());
            }
            let same_session = self.session.as_ref().map(|s| &s.id) == message.session_id.as_ref();
            if same_session && self.sequence.is_some_and(|seq| message.sequence < seq) {
                return Ok(false);
            }
            self.error = snapshot.as_ref().and_then(|s| s.error.clone());
            self.session = snapshot.map(|session| *session);
        } else {
            let Some(session) = &mut self.session else {
                return Ok(false);
            };
            if Some(&session.id) != message.session_id.as_ref()
                || self.sequence.is_some_and(|seq| message.sequence <= seq)
            {
                return Ok(false);
            }
            if self.sequence.is_some_and(|seq| message.sequence != seq + 1) {
                return Err(
                    "Adapter event sequence has a gap; reconnecting for a snapshot.".into(),
                );
            }
            match message.event {
                Event::View(view) => {
                    if session.mode() != Mode::Live {
                        return Err("Replay cannot change acquisition selection.".into());
                    }
                    validate_usage_window(view.usage_window)?;
                    session.live_view = Some(view);
                }
                Event::UsageWindow { polls } => {
                    if session.mode() != Mode::Live {
                        return Err("Replay cannot change Live CPU averaging.".into());
                    }
                    validate_usage_window(polls)?;
                    session.usage_window = Some(polls);
                }
                Event::HeapDetail(detail) => {
                    if session.mode() != Mode::Live {
                        return Err("Replay cannot change live heap capture.".into());
                    }
                    detail.validate()?;
                    *session.heap_detail = detail;
                }
                Event::Frame(frame) => {
                    frame.validate(session.mode())?;
                    if session.mode() == Mode::Live && session.inventory.is_none() {
                        let mut groups =
                            serde_json::to_value(&frame.groups).map_err(|e| e.to_string())?;
                        if let Value::Object(groups) = &mut groups {
                            groups.retain(|key, _| {
                                matches!(
                                    key.as_str(),
                                    "threads"
                                        | "heaps"
                                        | "semaphores"
                                        | "mutexes"
                                        | "msgqs"
                                        | "mem_slabs"
                                        | "workqs"
                                )
                            });
                        }
                        session.inventory = Some(
                            serde_json::from_value(groups)
                                .map_err(|e| format!("Invalid discovery inventory: {e}"))?,
                        );
                    }
                    session.frames_received += 1;
                    session.last_frame = Some(frame);
                    session.read_warning = false;
                }
                Event::Status(update) => {
                    session.status = update.status;
                    session.error = update.error.clone();
                    self.error = update.error;
                    if !session.status.is_active() || session.status == Status::Stopping {
                        session.samples_per_second = None;
                    }
                }
                Event::Notice { message } => {
                    session.last_notice = Some(message);
                    session.read_warning = true;
                }
                Event::Health { samples_per_second } => {
                    if samples_per_second.is_some_and(|v| !v.is_finite() || v < 0.) {
                        return Err("Invalid observed sample rate.".into());
                    }
                    session.samples_per_second = samples_per_second;
                }
                Event::Capabilities(capabilities) => session.capabilities = Some(capabilities),
                Event::Snapshot(_) => unreachable!(),
            }
        }
        self.sequence = Some(message.sequence);
        self.connection = Connection::Connected;
        Ok(true)
    }
}

pub fn percentage(value: Option<f64>) -> String {
    value
        .filter(|value| value.is_finite())
        .map(|value| format!("{value:.2}%"))
        .unwrap_or_else(|| "—".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn envelope(kind: &str, sequence: u64, session_id: &str, data: Value) -> Envelope {
        serde_json::from_value(json!({
            "version": PROTOCOL_VERSION, "type": kind, "sequence": sequence,
            "session_id": session_id, "data": data
        }))
        .unwrap()
    }

    fn snapshot(sequence: u64, id: &str) -> Envelope {
        envelope(
            "snapshot",
            sequence,
            id,
            json!({
                "id": id, "source": {"mode": "replay", "recording": "capture.ndjson.gz"},
                "elf": "zephyr.elf", "status": "opening", "frames_received": 0,
                "last_frame": null, "last_notice": null, "error": null
            }),
        )
    }

    fn frame(sequence: u64) -> Envelope {
        envelope(
            "frame",
            sequence,
            "one",
            json!({
                "decoded_at": 123.5,
                "groups": {
                    "threads": [{
                        "address": 536871096, "name": "worker", "is_idle": false,
                        "runtime": {
                            "cpu_normalized": 1.25, "stack_watermark_percent": null,
                            "is_active": true
                        },
                        "line": "thread worker", "glance": {"level": "busy"}
                    }],
                    "heaps": [{"name": "heap", "allocated_bytes": 16}]
                }
            }),
        )
    }

    #[test]
    fn typed_frames_preserve_other_serialized_fields_and_missing_values() {
        let mut state = ClientState::default();
        state.accept(snapshot(0, "one")).unwrap();
        state.accept(frame(1)).unwrap();
        let session = state.session.unwrap();
        assert_eq!(session.frames_received, 1);
        let frame = session.last_frame.unwrap();
        let runtime = frame.groups.threads[0].runtime.as_ref().unwrap();
        assert_eq!(percentage(runtime.cpu_normalized), "1.25%");
        assert_eq!(percentage(runtime.stack_watermark_percent), "—");
        let json = serde_json::to_value(frame).unwrap();
        assert_eq!(json["groups"]["heaps"][0]["allocated_bytes"], 16);
        assert_eq!(json["groups"]["threads"][0]["runtime"]["is_active"], true);
        assert_eq!(json["groups"]["threads"][0]["glance"]["level"], "busy");
    }

    #[test]
    fn legacy_idle_identity_is_not_changed_by_the_wire_round_trip() {
        let frame: Frame = serde_json::from_value(json!({
            "decoded_at": 1.0,
            "groups": {"threads": [{"address": 1, "name": "idle", "runtime": null}]}
        }))
        .unwrap();
        let groups = serde_json::to_value(frame.groups).unwrap();
        assert!(groups["threads"][0].get("is_idle").is_none());
        let parsed = crate::model::parse(&groups.to_string()).unwrap();
        assert!(parsed[0].visible_threads().is_empty());
    }

    #[test]
    fn duplicate_stale_and_wrong_session_events_do_not_change_state() {
        let mut state = ClientState::default();
        state.accept(snapshot(0, "one")).unwrap();
        state.accept(frame(1)).unwrap();
        assert!(!state.accept(frame(1)).unwrap());
        assert!(!state.accept(snapshot(0, "one")).unwrap());
        let old = envelope("status", 2, "other", json!({"status": "failed"}));
        assert!(!state.accept(old).unwrap());
        assert_eq!(state.session.as_ref().unwrap().frames_received, 1);
        assert_eq!(state.session.as_ref().unwrap().status, Status::Opening);
    }

    #[test]
    fn snapshots_restore_count_and_new_sessions_reset_it() {
        let mut state = ClientState::default();
        state.accept(snapshot(0, "one")).unwrap();
        state.accept(frame(1)).unwrap();
        state.connection = Connection::Retrying;
        let mut restored = snapshot(20, "one");
        if let Event::Snapshot(Some(session)) = &mut restored.event {
            session.frames_received = 18;
            session.status = Status::Ended;
        }
        state.accept(restored).unwrap();
        assert_eq!(state.connection, Connection::Connected);
        assert_eq!(state.session.as_ref().unwrap().frames_received, 18);
        assert!(state.can_open());
        state.accept(snapshot(0, "two")).unwrap();
        assert_eq!(state.session.as_ref().unwrap().frames_received, 0);
        assert!(!state.can_open());
        assert!(state.can_stop());
    }

    #[test]
    fn protocol_mismatch_gap_and_inconsistent_snapshot_are_rejected() {
        let mut state = ClientState::default();
        let mut wrong_version = snapshot(0, "one");
        wrong_version.version = 1;
        assert!(state.accept(wrong_version).is_err());
        state.accept(snapshot(0, "one")).unwrap();
        assert!(state.accept(frame(2)).is_err());
        assert_eq!(state.session.as_ref().unwrap().frames_received, 0);
        let mut wrong_id = snapshot(1, "one");
        wrong_id.session_id = Some("two".into());
        assert!(state.accept(wrong_id).is_err());
    }

    #[test]
    fn status_notice_empty_snapshot_and_controls_follow_contract() {
        let mut state = ClientState::default();
        assert!(!state.can_open());
        assert!(!state.can_stop());
        state
            .accept(
                serde_json::from_value(json!({
                    "version": PROTOCOL_VERSION, "type": "snapshot", "session_id": null,
                    "sequence": 0, "data": null
                }))
                .unwrap(),
            )
            .unwrap();
        assert!(state.can_open());
        state.accept(snapshot(0, "one")).unwrap();
        state
            .accept(envelope(
                "notice",
                1,
                "one",
                json!({"message": "read fault"}),
            ))
            .unwrap();
        state
            .accept(envelope(
                "status",
                2,
                "one",
                json!({
                    "status": "failed", "error": "Wrong ELF"
                }),
            ))
            .unwrap();
        assert_eq!(state.error.as_deref(), Some("Wrong ELF"));
        assert_eq!(
            state.session.as_ref().unwrap().last_notice.as_deref(),
            Some("read fault")
        );
        assert!(state.can_open());
        assert!(!state.can_stop());
        state.pending = true;
        assert!(!state.can_open());
    }

    #[test]
    fn unknown_events_statuses_and_malformed_frames_fail_deserialization() {
        for (kind, data) in [
            ("unknown", json!({})),
            ("status", json!({"status": "unknown"})),
            ("frame", json!({"groups": {}})),
        ] {
            assert!(
                serde_json::from_value::<Envelope>(json!({
                    "version": PROTOCOL_VERSION, "type": kind, "session_id": "one",
                    "sequence": 1, "data": data
                }))
                .is_err()
            );
        }
    }

    #[test]
    fn session_card_never_invents_a_board_connection_or_sample_rate() {
        let labels = session_labels(None, Connection::Connected);
        assert_eq!(labels[0], "OPEN SESSION");
        assert!(labels[1].contains("open a recording"));
        let message = snapshot(0, "one");
        let Event::Snapshot(Some(session)) = message.event else {
            panic!("expected a session");
        };
        let labels = session_labels(Some(&session), Connection::Connected);
        assert_eq!(labels[1], "Board unknown · capture.ndjson.gz");
        assert_eq!(labels[2], "0 decoded frames");
        assert!(!labels.join(" ").contains("samples/s"));
        assert!(!labels.join(" ").contains("Live"));
        let retrying = session_labels(Some(&session), Connection::Retrying);
        assert!(retrying[2].contains("disconnected"));
    }

    #[test]
    fn native_file_choice_distinguishes_cancellation_from_a_selected_path() {
        let choice: FileChoice =
            serde_json::from_value(json!({"path": "build/zephyr.elf"})).unwrap();
        assert_eq!(choice.path.as_deref(), Some("build/zephyr.elf"));
        let cancelled: FileChoice = serde_json::from_value(json!({"path": null})).unwrap();
        assert!(cancelled.path.is_none());
        assert_eq!(serde_json::to_value(PickerKind::Elf).unwrap(), "elf");
        assert!(serde_json::from_value::<FileChoice>(json!({"path": 12})).is_err());
    }

    fn live_snapshot(id: &str) -> Envelope {
        envelope(
            "snapshot",
            0,
            id,
            json!({
                "id": id, "source": {
                    "mode": "live", "configuration": {"build_dir": "build"},
                    "runner": "jlink", "target": "nRF5340", "frequency": 4000000
                },
                "elf": "build/zephyr/zephyr.elf", "status": "opening",
                "frames_received": 0, "last_frame": null, "last_notice": null, "error": null
            }),
        )
    }

    #[test]
    fn fast_timing_accepts_zero_polling_without_claiming_an_achieved_rate() {
        let timing = AcquisitionTiming {
            period: 0.,
            heap_period: 0.1,
            heap_cooldown: 0.,
        };
        assert!(timing.validate().is_ok());
        assert!(timing.polling_label().contains("back-to-back"));
        assert!(timing.heap_label().contains("0.1 s"));
        let mut snapshot = live_snapshot("fast");
        if let Event::Snapshot(Some(session)) = &mut snapshot.event {
            session.timing = Some(Box::new(timing));
        }
        let mut state = ClientState::default();
        state.accept(snapshot).unwrap();
        assert_eq!(
            state
                .session
                .as_ref()
                .unwrap()
                .timing
                .as_ref()
                .unwrap()
                .period,
            0.
        );
        for (period, heap_period, heap_cooldown) in [
            (-1., 0.1, 0.),
            (0., 0., 0.),
            (0., 0.1, -1.),
            (f64::NAN, 0.1, 0.),
            (0., f64::INFINITY, 0.),
        ] {
            assert!(
                AcquisitionTiming {
                    period,
                    heap_period,
                    heap_cooldown
                }
                .validate()
                .is_err()
            );
        }
    }

    #[test]
    fn cpu_window_tracks_worker_ack_and_rejects_invalid_or_replay_updates() {
        let mut state = ClientState::default();
        state.accept(live_snapshot("one")).unwrap();
        assert!(
            state
                .accept(envelope("usage_window", 1, "one", json!({"polls": 16})))
                .unwrap()
        );
        assert_eq!(state.session.as_ref().unwrap().usage_window, Some(16));
        assert!(
            !state
                .accept(envelope("usage_window", 1, "one", json!({"polls": 32})))
                .unwrap()
        );
        for polls in [0, 65] {
            assert!(
                state
                    .accept(envelope("usage_window", 2, "one", json!({"polls": polls})))
                    .is_err()
            );
            assert_eq!(state.session.as_ref().unwrap().usage_window, Some(16));
        }
        let mut restored = live_snapshot("one");
        if let Event::Snapshot(Some(session)) = &mut restored.event {
            session.usage_window = Some(32);
        }
        restored.sequence = 2;
        state.accept(restored).unwrap();
        assert_eq!(state.session.as_ref().unwrap().usage_window, Some(32));
        let mut replay = ClientState::default();
        replay.accept(snapshot(0, "replay")).unwrap();
        assert!(
            replay
                .accept(envelope("usage_window", 1, "replay", json!({"polls": 16})))
                .is_err()
        );
    }

    #[test]
    fn heap_interest_serializes_navigation_and_releases_only_after_the_pending_reply() {
        let heap = HeapIdentity {
            address: 4096,
            name: "heap".into(),
        };
        let mut interest = HeapInterest::default();
        assert!(interest.begin(Some(heap.clone()), None).is_none());
        interest.bind(Some("live"));
        assert!(interest.begin(None, None).is_none());
        let opening = interest.begin(Some(heap.clone()), None).unwrap();
        assert!(interest.begin(None, None).is_none());
        interest.finish(opening, None, false);
        let releasing = interest.begin(None, Some(&heap)).unwrap();
        assert!(interest.begin(Some(heap.clone()), Some(&heap)).is_none());
        interest.finish(releasing, None, false);
        assert!(interest.begin(Some(heap), None).is_some());
    }

    #[test]
    fn old_heap_replies_cannot_block_or_report_errors_in_a_new_session_or_connection() {
        let heap = HeapIdentity {
            address: 4096,
            name: "heap".into(),
        };
        let mut interest = HeapInterest::default();
        interest.bind(Some("old"));
        let old = interest.begin(Some(heap.clone()), None).unwrap();
        interest.bind(Some("new"));
        let new = interest.begin(Some(heap.clone()), None).unwrap();
        interest.finish(old, Some("Old session failed".into()), true);
        assert!(interest.error.is_none());
        assert!(interest.begin(None, None).is_none());
        interest.finish(new, None, false);
        interest.reset();
        interest.bind(Some("new"));
        let reconnected = interest.begin(Some(heap), None).unwrap();
        interest.finish(new, Some("Old connection failed".into()), true);
        assert!(interest.error.is_none());
        interest.finish(reconnected, None, false);
        assert!(interest.begin(None, None).is_some());
    }

    #[test]
    fn heap_conflicts_retry_after_release_or_join_but_other_errors_do_not_loop() {
        let requested = HeapIdentity {
            address: 4096,
            name: "requested".into(),
        };
        let other = HeapIdentity {
            address: 8192,
            name: "other".into(),
        };
        let mut interest = HeapInterest::default();
        interest.bind(Some("live"));
        let blocked = interest
            .begin(Some(requested.clone()), Some(&other))
            .unwrap();
        interest.finish(blocked, Some("Busy".into()), true);
        assert!(
            interest
                .begin(Some(requested.clone()), Some(&other))
                .is_none()
        );
        // The release may already have arrived before the HTTP error.
        let retry = interest.begin(Some(requested.clone()), None).unwrap();
        interest.finish(retry, Some("Busy".into()), true);
        // Another tab may claim the requested heap before a retry: sharing is allowed.
        let joining = interest
            .begin(Some(requested.clone()), Some(&requested))
            .unwrap();
        interest.finish(joining, Some("Invalid request".into()), false);
        assert!(interest.begin(Some(requested), None).is_none());
    }

    #[test]
    fn view_conflicts_retry_after_owner_release_regardless_of_the_retained_screen() {
        let desired = LiveView::Overview { heap: None };
        for release_before_reply in [false, true] {
            let mut state = LiveViewState {
                revision: 4,
                owner: Some("winner".into()),
                view: LiveView::Objects {
                    kind: None,
                    query: String::new(),
                },
                usage_window: 16,
            };
            let mut interest = ViewInterest::default();
            interest.bind(Some("live"));
            let attempt = interest.begin(Some(desired.clone()), None).unwrap();
            if release_before_reply {
                state.owner = None;
                // No concurrent retry while the original HTTP request is pending.
                assert!(
                    interest
                        .begin(Some(desired.clone()), state.active_view())
                        .is_none()
                );
            }
            interest.finish(
                attempt,
                Some("Only one tab controls Live acquisition.".into()),
                true,
            );
            if !release_before_reply {
                assert!(
                    interest
                        .begin(Some(desired.clone()), state.active_view())
                        .is_none()
                );
                state.owner = None;
            }
            assert_ne!(state.view, desired); // The server keeps the winner's screen.
            let retry = interest
                .begin(Some(desired.clone()), state.active_view())
                .unwrap();
            assert!(interest.error.is_none());
            assert!(
                interest
                    .begin(Some(desired.clone()), state.active_view())
                    .is_none()
            );
            state.owner = Some("waiting-tab".into());
            state.view = desired.clone();
            interest.finish(retry, None, false);
            assert!(
                interest
                    .begin(Some(desired.clone()), state.active_view())
                    .is_none()
            );
        }
    }

    #[test]
    fn heap_capture_is_identity_checked_and_does_not_invent_normal_frames() {
        let mut state = ClientState::default();
        state.accept(live_snapshot("live")).unwrap();
        let mut detail = json!({
            "heap": {"address": 4096, "name": "packet_heap"},
            "last_map": {
                "heap": {"address": 4096, "name": "packet_heap"},
                "chunks": [{"size": 64, "used": false}],
                "captured_at": 123.0, "live_elapsed": 1.2, "frame_number": 12,
            }, "error": null,
        });
        state
            .accept(envelope("heap_detail", 1, "live", detail.clone()))
            .unwrap();
        let session = state.session.as_ref().unwrap();
        assert_eq!(session.frames_received, 0);
        assert!(session.last_frame.is_none());
        let map = session.heap_detail.last_map.as_ref().unwrap();
        assert_eq!(map.frame_number, 12);
        assert_eq!(map.chunks[0].size, 64);
        detail["last_map"]["heap"]["address"] = json!(8192);
        assert!(
            state
                .accept(envelope("heap_detail", 2, "live", detail))
                .is_err()
        );
        state.accept(live_snapshot("replacement")).unwrap();
        assert!(
            state
                .session
                .as_ref()
                .unwrap()
                .heap_detail
                .last_map
                .is_none()
        );
        assert!(
            !state
                .accept(envelope("heap_detail", 3, "live", json!({})))
                .unwrap()
        );
    }

    #[test]
    fn live_status_rate_and_transport_are_independent() {
        let mut state = ClientState::default();
        state.accept(live_snapshot("live")).unwrap();
        assert!(state.is_live());
        assert!(state.can_stop());
        assert!(!state.can_reconnect());
        let labels = session_labels(state.session.as_ref(), state.connection);
        assert_eq!(labels[0], "LIVE · Connecting");
        assert_eq!(labels[1], "Board unknown · J-Link");
        assert_eq!(labels[2], "Not sampling");
        state
            .accept(envelope("status", 1, "live", json!({"status": "reading"})))
            .unwrap();
        state
            .accept(envelope(
                "health",
                2,
                "live",
                json!({"samples_per_second": 9.8}),
            ))
            .unwrap();
        assert_eq!(
            session_labels(state.session.as_ref(), state.connection)[2],
            "9.8 samples/s"
        );
        state.connection = Connection::Retrying;
        assert!(!state.can_stop());
        assert!(
            session_labels(state.session.as_ref(), state.connection)[2]
                .contains("Adapter disconnected")
        );
        assert_eq!(state.session.as_ref().unwrap().status, Status::Reading);
        state
            .accept(envelope(
                "status",
                3,
                "live",
                json!({"status": "lost", "error": "probe removed"}),
            ))
            .unwrap();
        assert!(state.can_reconnect());
        assert_eq!(state.session.as_ref().unwrap().samples_per_second, None);
        let labels = session_labels(state.session.as_ref(), state.connection);
        assert_eq!(labels[0], "LIVE · Connection lost");
        assert!(!labels.join(" ").contains("Last sample"));
        state.accept(live_snapshot("new")).unwrap();
        assert_eq!(state.session.as_ref().unwrap().frames_received, 0);
    }

    #[test]
    fn header_indicator_requires_fresh_acquisition_not_just_a_socket() {
        let mut state = ClientState::default();
        assert_eq!(state.indicator(false), SessionIndicator::Busy);
        state.connection = Connection::Connected;
        assert_eq!(state.indicator(true), SessionIndicator::Idle);
        state.accept(live_snapshot("live")).unwrap();
        assert_eq!(state.indicator(true), SessionIndicator::Busy);
        state
            .accept(envelope("status", 1, "live", json!({"status":"reading"})))
            .unwrap();
        assert_eq!(state.indicator(false), SessionIndicator::Waiting);
        assert_eq!(state.indicator(true), SessionIndicator::Healthy);
        state.session.as_mut().unwrap().read_warning = true;
        assert_eq!(state.indicator(true), SessionIndicator::Warning);
        state.session.as_mut().unwrap().read_warning = false;
        state.pending = true;
        assert_eq!(state.indicator(true), SessionIndicator::Busy);
        state.pending = false;
        state.connection = Connection::Retrying;
        assert_eq!(state.indicator(true), SessionIndicator::Idle);
        state
            .accept(envelope("status", 2, "live", json!({"status":"lost"})))
            .unwrap();
        assert_eq!(state.indicator(true), SessionIndicator::Failed);
        state
            .accept(envelope("status", 3, "live", json!({"status":"stopped"})))
            .unwrap();
        assert_eq!(state.indicator(true), SessionIndicator::Idle);
    }

    #[test]
    fn warning_survives_a_snapshot_and_only_a_new_frame_clears_it() {
        let mut snapshot = live_snapshot("live");
        if let Event::Snapshot(Some(session)) = &mut snapshot.event {
            session.status = Status::Reading;
            session.read_warning = true;
        }
        let mut state = ClientState::default();
        state.accept(snapshot).unwrap();
        assert_eq!(state.indicator(true), SessionIndicator::Warning);
        state
            .accept(envelope(
                "frame",
                1,
                "live",
                json!({
                    "decoded_at": 1., "live_elapsed": 0., "groups": {"threads": []}
                }),
            ))
            .unwrap();
        assert_eq!(state.indicator(true), SessionIndicator::Healthy);
    }

    #[test]
    fn header_exposes_only_the_applicable_acquisition_action() {
        let mut state = ClientState {
            connection: Connection::Connected,
            ..Default::default()
        };
        assert_eq!(state.primary_action(), SessionAction::Connect);
        assert!(state.can_primary_action());
        state.accept(live_snapshot("live")).unwrap();
        assert_eq!(state.primary_action(), SessionAction::Disconnect);
        assert!(state.can_primary_action());
        state
            .accept(envelope("status", 1, "live", json!({"status":"stopping"})))
            .unwrap();
        assert!(!state.can_primary_action());
        state
            .accept(envelope("status", 2, "live", json!({"status":"stopped"})))
            .unwrap();
        assert_eq!(state.primary_action(), SessionAction::Reconnect);
        assert!(state.can_primary_action());
        state.pending = true;
        assert!(!state.can_primary_action());
        state.pending = false;
        state.accept(snapshot(0, "replay")).unwrap();
        assert_eq!(state.primary_action(), SessionAction::StopReplay);
        state
            .accept(envelope("status", 1, "replay", json!({"status":"ended"})))
            .unwrap();
        assert_eq!(state.primary_action(), SessionAction::OpenReplay);
        assert!(state.can_primary_action());
        state.connection = Connection::Retrying;
        assert!(!state.can_primary_action());
    }

    #[test]
    fn live_frames_need_live_time_but_replay_never_uses_it() {
        let mut live = ClientState::default();
        live.accept(live_snapshot("live")).unwrap();
        assert!(
            live.accept(envelope(
                "frame",
                1,
                "live",
                json!({"decoded_at": 1., "groups": {"threads": []}})
            ))
            .is_err()
        );
        live.accept(envelope(
            "frame",
            1,
            "live",
            json!({"decoded_at": 1., "live_elapsed": 0., "groups": {"threads": []}}),
        ))
        .unwrap();
        assert_eq!(live.session.as_ref().unwrap().frames_received, 1);
        assert!(
            live.accept(envelope(
                "health",
                2,
                "live",
                json!({"samples_per_second": -1.})
            ))
            .is_err()
        );
        let mut replay = ClientState::default();
        replay.accept(snapshot(0, "one")).unwrap();
        assert!(
            replay
                .accept(envelope(
                    "frame",
                    1,
                    "one",
                    json!({"decoded_at": 1., "live_elapsed": 0., "groups": {"threads": []}})
                ))
                .is_err()
        );
    }

    #[test]
    fn resolved_context_survives_snapshots_and_git_is_not_build_provenance() {
        let mut message = serde_json::to_value(json!({
            "id": "one", "source": {"mode": "replay", "recording": "/captures/session.ndjson.gz"},
            "elf": "build/zephyr/zephyr.elf", "status": "ended", "frames_received": 20,
            "last_frame": null, "last_notice": null, "error": null,
            "context": {
                "build_dir": "build", "image_dir": "build/app", "domain": "app",
                "domains": ["app", "network"], "elf": "build/app/zephyr/zephyr.elf",
                "board": "nrf5340dk/nrf5340/cpuapp", "project": "application",
                "source_dir": "application", "runner": null, "target": null,
                "frequency": null, "warnings": [],
                "git": {"commit": "abc123", "branch": null, "dirty": true,
                        "provenance": "current_checkout"}
            }
        }))
        .unwrap();
        let session: Session = serde_json::from_value(message.take()).unwrap();
        let labels = session_labels(Some(&session), Connection::Connected);
        assert_eq!(labels[0], "REPLAY · ended");
        assert_eq!(labels[1], "nrf5340dk/nrf5340/cpuapp · session.ndjson.gz");
        assert_eq!(labels[3], "application / build");
        let tooltip = session.context.unwrap().tooltip();
        assert!(tooltip.contains("detached HEAD"));
        assert!(tooltip.contains("tracked changes"));
        assert!(tooltip.contains("not proof"));
    }
}

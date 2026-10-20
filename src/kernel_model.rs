//! Typed dump fields and the shared, TUI-compatible kernel overview row.
use crate::model::{Frame, percentage};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct Object {
    pub address: u64,
    pub name: String,
    pub waiters: Option<Vec<String>>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Semaphore {
    #[serde(flatten)]
    pub object: Object,
    pub count: Option<u64>,
    pub limit: Option<u64>,
}

/// Static objects are matched by address and name, never by filtered row index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SemaphoreKey {
    pub address: u64,
    pub name: String,
}

impl SemaphoreKey {
    pub fn in_frame<'a>(&self, frame: &'a Frame) -> Option<&'a Semaphore> {
        frame
            .semaphores
            .iter()
            .find(|sem| sem.object.address == self.address && sem.object.name == self.name)
    }
}

impl Semaphore {
    pub fn limit_label(&self) -> String {
        match self.limit {
            Some(0) => "Not initialized".into(),
            Some(0xffff_ffff) => "Unlimited".into(),
            limit => number(limit),
        }
    }

    pub fn fill(&self) -> Option<f64> {
        match self.limit {
            Some(0 | 0xffff_ffff) => None,
            limit => ratio(self.count, limit),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Mutex {
    #[serde(flatten)]
    pub object: Object,
    pub lock_count: Option<u64>,
    pub owner_address: Option<u64>,
    pub owner_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MutexKey {
    pub address: u64,
    pub name: String,
}

impl MutexKey {
    pub fn in_frame<'a>(&self, frame: &'a Frame) -> Option<&'a Mutex> {
        frame
            .mutexes
            .iter()
            .find(|mutex| mutex.object.address == self.address && mutex.object.name == self.name)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MutexState {
    Free,
    Locked,
    Contended,
    Unknown,
}

impl Mutex {
    pub fn state(&self) -> MutexState {
        match self.owner_address {
            Some(0) => MutexState::Free,
            Some(_) => match self.object.waiters.as_deref() {
                Some([]) => MutexState::Locked,
                Some(_) => MutexState::Contended,
                None => MutexState::Unknown,
            },
            None => MutexState::Unknown,
        }
    }

    pub fn owner_label(&self) -> String {
        match self.owner_address {
            Some(0) => "None · Free".into(),
            Some(address) => match self.owner_name.as_deref().filter(|name| !name.is_empty()) {
                Some(name) => format!("{name} · 0x{address:08x}"),
                None => format!("0x{address:08x}"),
            },
            None => "—".into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct MessageQueue {
    #[serde(flatten)]
    pub object: Object,
    pub used_msgs: Option<u64>,
    pub max_msgs: Option<u64>,
    pub msg_size: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageQueueKey {
    pub address: u64,
    pub name: String,
}

impl MessageQueueKey {
    pub fn in_frame<'a>(&self, frame: &'a Frame) -> Option<&'a MessageQueue> {
        frame
            .msgqs
            .iter()
            .find(|queue| queue.object.address == self.address && queue.object.name == self.name)
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct MemorySlab {
    #[serde(flatten)]
    pub object: Object,
    pub num_used: Option<u64>,
    pub num_blocks: Option<u64>,
    pub block_size: Option<u64>,
    pub max_used: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemorySlabKey {
    pub address: u64,
    pub name: String,
}

impl MemorySlabKey {
    pub fn in_frame<'a>(&self, frame: &'a Frame) -> Option<&'a MemorySlab> {
        frame
            .mem_slabs
            .iter()
            .find(|slab| slab.object.address == self.address && slab.object.name == self.name)
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct WorkQueue {
    #[serde(flatten)]
    pub object: Object,
    pub flags: Option<u64>,
    pub thread_name: Option<String>,
    pub pending: Option<Vec<String>>,
    #[serde(default)]
    pub pending_truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkQueueKey {
    pub address: u64,
    pub name: String,
}

impl WorkQueueKey {
    pub fn in_frame<'a>(&self, frame: &'a Frame) -> Option<&'a WorkQueue> {
        frame
            .workqs
            .iter()
            .find(|queue| queue.object.address == self.address && queue.object.name == self.name)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Heap,
    MemorySlab,
    MessageQueue,
    Mutex,
    Semaphore,
    WorkQueue,
}

impl Kind {
    pub const ALL: [Self; 6] = [
        Self::Heap,
        Self::MemorySlab,
        Self::MessageQueue,
        Self::Mutex,
        Self::Semaphore,
        Self::WorkQueue,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Heap => "Heap",
            Self::MemorySlab => "Memory slab",
            Self::MessageQueue => "Message queue",
            Self::Mutex => "Mutex",
            Self::Semaphore => "Semaphore",
            Self::WorkQueue => "Work queue",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Normal,
    Busy,
    Blocked,
}

pub struct Row<'a> {
    pub kind: Kind,
    pub name: &'a str,
    pub address: u64,
    pub summary: String,
    pub fill: Option<f64>,
    pub peak: Option<f64>,
    pub waiters: Option<&'a [String]>,
    pub state: State,
    // Distinct from the row color: a handler may be running with no backlog.
    in_use: bool,
}

impl<'a> Row<'a> {
    fn new(kind: Kind, object: &'a Object) -> Self {
        Self {
            kind,
            name: &object.name,
            address: object.address,
            summary: "—".into(),
            fill: None,
            peak: None,
            waiters: object.waiters.as_deref(),
            in_use: false,
            state: if object.waiters.as_ref().is_some_and(|w| !w.is_empty()) {
                State::Busy
            } else {
                State::Normal
            },
        }
    }

    pub fn matches(&self, kind: Option<Kind>, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        kind.is_none_or(|kind| self.kind == kind)
            && (self.name.to_lowercase().contains(&query)
                || format!("0x{:x}", self.address).contains(&query)
                || format!("0x{:08x}", self.address).contains(&query))
    }
}

/// Aggregate the displayed rows using KernelObjectListView._aggregate's rules.
/// Pressure is a union, not a sum or an average of unrelated usage bars.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub total: usize,
    pub pressured: usize,
    pub contended: usize,
    pub with_waiters: usize,
    pub waiting_threads: usize,
    pub unknown_wait_queues: usize,
}

impl Summary {
    pub fn from_rows(rows: &[&Row<'_>]) -> Self {
        let mut summary = Self::default();
        for row in rows {
            summary.total += 1;
            let has_waiters = row.waiters.is_some_and(|waiters| !waiters.is_empty());
            let contended = row.state == State::Blocked;
            summary.contended += usize::from(contended);
            summary.with_waiters += usize::from(has_waiters);
            summary.pressured += usize::from(has_waiters || row.in_use || contended);
            match row.waiters {
                Some(waiters) => summary.waiting_threads += waiters.len(),
                None => summary.unknown_wait_queues += 1,
            }
        }
        summary
    }

    pub fn percent(&self) -> f64 {
        if self.total == 0 {
            0.
        } else {
            self.pressured as f64 / self.total as f64 * 100.
        }
    }

    pub fn label(kind: Option<Kind>) -> &'static str {
        match kind {
            None => "All Objects",
            Some(Kind::Heap) => "All Heaps",
            Some(Kind::MemorySlab) => "All Slabs",
            Some(Kind::MessageQueue) => "All Queues",
            Some(Kind::Mutex) => "All Mutexes",
            Some(Kind::Semaphore) => "All Semaphores",
            Some(Kind::WorkQueue) => "All Work Queues",
        }
    }

    pub fn state_label(&self, kind: Option<Kind>) -> String {
        let word = match kind {
            Some(Kind::Heap | Kind::MemorySlab) => "exhausted",
            Some(Kind::WorkQueue) => "refusing",
            _ => "contended",
        };
        format!(
            "{} objects | {} {word} | {} with waiters",
            self.total, self.contended, self.with_waiters
        )
    }

    pub fn waiters_label(&self) -> String {
        if self.unknown_wait_queues == 0 {
            self.waiting_threads.to_string()
        } else {
            format!("{}+?", self.waiting_threads)
        }
    }
}

fn number(value: Option<u64>) -> String {
    value.map(|v| v.to_string()).unwrap_or_else(|| "—".into())
}

fn ratio(used: Option<u64>, size: Option<u64>) -> Option<f64> {
    percentage(used?, size?)
}

/// State and labels mirror TUIKernelObjectInfo.row_values. Missing fields stay
/// unknown, rather than being presented as zero, free, or an empty wait queue.
pub fn rows(frame: &Frame) -> Vec<Row<'_>> {
    let mut rows = Vec::new();
    for heap in &frame.heaps {
        rows.push(Row {
            kind: Kind::Heap,
            name: &heap.name,
            address: heap.address,
            summary: format!("{} / {} B", heap.allocated_bytes, heap.total()),
            fill: heap.percent(),
            peak: None,
            waiters: heap.waiters.as_deref(),
            in_use: false,
            state: if heap.total() > 0 && heap.free_bytes == 0 {
                State::Blocked
            } else if heap.waiters.as_ref().is_some_and(|w| !w.is_empty()) {
                State::Busy
            } else {
                State::Normal
            },
        });
    }
    for slab in &frame.mem_slabs {
        let mut row = Row::new(Kind::MemorySlab, &slab.object);
        row.summary = format!(
            "{} / {} × {} B",
            number(slab.num_used),
            number(slab.num_blocks),
            number(slab.block_size)
        );
        row.fill = ratio(slab.num_used, slab.num_blocks);
        row.peak = ratio(slab.max_used, slab.num_blocks);
        if matches!((slab.num_used, slab.num_blocks), (Some(u), Some(n)) if n > 0 && u >= n) {
            row.state = State::Blocked;
        }
        rows.push(row);
    }
    for queue in &frame.msgqs {
        let mut row = Row::new(Kind::MessageQueue, &queue.object);
        row.summary = format!(
            "{} / {} × {} B",
            number(queue.used_msgs),
            number(queue.max_msgs),
            number(queue.msg_size)
        );
        row.fill = ratio(queue.used_msgs, queue.max_msgs);
        if matches!((queue.used_msgs, queue.max_msgs), (Some(u), Some(n)) if n > 0 && u >= n) {
            row.state = State::Blocked;
        }
        rows.push(row);
    }
    for mutex in &frame.mutexes {
        let mut row = Row::new(Kind::Mutex, &mutex.object);
        match mutex.owner_address {
            Some(0) => {
                row.summary = "FREE".into();
                row.fill = Some(0.);
                row.state = State::Normal;
            }
            Some(address) => {
                let owner = mutex
                    .owner_name
                    .clone()
                    .unwrap_or_else(|| format!("0x{address:x}"));
                row.summary = format!("LOCKED · {owner}");
                if let Some(count) = mutex.lock_count.filter(|count| *count > 1) {
                    row.summary.push_str(&format!(" ×{count}"));
                }
                row.fill = Some(100.);
                row.in_use = true;
                row.state = if row.waiters.is_some_and(|w| !w.is_empty()) {
                    State::Blocked
                } else {
                    State::Busy
                };
            }
            None => {}
        }
        rows.push(row);
    }
    for semaphore in &frame.semaphores {
        let mut row = Row::new(Kind::Semaphore, &semaphore.object);
        row.summary = match semaphore.limit {
            Some(0) => "NOT INITIALIZED".into(),
            Some(0xffff_ffff) => format!("{} / unlimited", number(semaphore.count)),
            limit => format!("{} / {}", number(semaphore.count), number(limit)),
        };
        row.fill = semaphore.fill();
        rows.push(row);
    }
    for queue in &frame.workqs {
        let mut row = Row::new(Kind::WorkQueue, &queue.object);
        if let Some(flags) = queue.flags {
            row.summary = if flags & 2 != 0 {
                "BUSY"
            } else if flags & 1 != 0 {
                "IDLE"
            } else {
                "NOT STARTED"
            }
            .into();
            row.fill = Some(if flags & 2 != 0 { 100. } else { 0. });
            if flags & 12 != 0 {
                row.state = State::Blocked;
            }
        }
        let depth = queue.pending.as_ref().map(Vec::len);
        row.in_use =
            queue.flags.is_some_and(|flags| flags & 2 != 0) || depth.is_some_and(|depth| depth > 0);
        let depth_label = depth
            .map(|n| format!("{n}{}", if queue.pending_truncated { "+" } else { "" }))
            .unwrap_or_else(|| "—".into());
        row.summary.push_str(&format!(" · {depth_label} pending"));
        if row.state == State::Normal && depth.is_some_and(|n| n > 0) {
            row.state = State::Busy;
        }
        rows.push(row);
    }
    rows.sort_by(|a, b| {
        a.kind
            .cmp(&b.kind)
            .then_with(|| a.name.cmp(b.name))
            .then_with(|| a.address.cmp(&b.address))
    });
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::parse;
    use serde_json::{Value, json};

    fn fixture() -> Vec<Frame> {
        parse(include_str!("../test-data/kernel-objects-minimal.json")).unwrap()
    }

    fn find<'r, 'f>(rows: &'r [Row<'f>], kind: Kind) -> &'r Row<'f> {
        rows.iter().find(|row| row.kind == kind).unwrap()
    }

    fn summary(frame: &Frame, kind: Option<Kind>, query: &str) -> Summary {
        let rows = rows(frame);
        Summary::from_rows(
            &rows
                .iter()
                .filter(|row| row.matches(kind, query))
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn aggregate_matches_tui_pressure_and_partial_waiter_totals() {
        let mut frames = fixture();
        let all = summary(&frames[0], None, "");
        assert_eq!(
            all,
            Summary {
                total: 6,
                pressured: 4, // Semaphore, held mutex, full msgq, and busy workq.
                contended: 2,
                with_waiters: 2,
                waiting_threads: 2, // Sum across queues, not unique thread names.
                unknown_wait_queues: 1,
            }
        );
        assert!((all.percent() - 200. / 3.).abs() < 0.00001);
        assert_eq!(
            all.state_label(None),
            "6 objects | 2 contended | 2 with waiters"
        );
        assert_eq!(all.waiters_label(), "2+?");
        frames[0].heaps[0].waiters = Some(vec![]);
        assert_eq!(summary(&frames[0], None, "").waiters_label(), "2");
        // A nonempty semaphore is not pressure, nor an average of usage bars.
        let next = summary(&frames[1], None, "");
        assert_eq!(
            next.state_label(None),
            "1 objects | 0 contended | 0 with waiters"
        );
        assert_eq!(next.percent(), 0.);
        assert_eq!(next.waiters_label(), "0");
    }

    #[test]
    fn aggregate_tracks_filters_and_retains_type_specific_wording() {
        let mut frames = fixture();
        let gate = summary(&frames[0], None, "gate");
        assert_eq!(gate.total, 2);
        assert_eq!(gate.pressured, 2); // A contended, held mutex counts only once.
        assert_eq!(gate.percent(), 100.);
        let sem = summary(&frames[0], Some(Kind::Semaphore), "GATE");
        assert_eq!(
            sem.state_label(Some(Kind::Semaphore)),
            "1 objects | 0 contended | 1 with waiters"
        );
        assert_eq!(Summary::label(Some(Kind::Semaphore)), "All Semaphores");
        assert_eq!(Summary::label(None), "All Objects");
        frames[0].heaps[0].free_bytes = 0;
        frames[0].mem_slabs[0].num_used = Some(8);
        frames[0].workqs[0].flags = Some(9);
        for kind in [Kind::Heap, Kind::MemorySlab, Kind::WorkQueue] {
            let filtered = summary(&frames[0], Some(kind), "");
            assert_eq!(filtered.contended, 1);
            let word = if kind == Kind::WorkQueue {
                "refusing"
            } else {
                "exhausted"
            };
            assert!(filtered.state_label(Some(kind)).contains(word));
        }
        let empty = summary(&frames[0], Some(Kind::Heap), "not-found");
        assert_eq!(empty, Summary::default());
        assert_eq!(empty.percent(), 0.);
        assert_eq!(empty.waiters_label(), "0");
        let frame = parse(r#"{"threads":[]}"#).unwrap().remove(0);
        assert_eq!(summary(&frame, None, ""), Summary::default());
    }

    #[test]
    fn running_handler_without_pending_work_and_uncontended_mutex_are_in_use() {
        let mut frames = fixture();
        frames[0].workqs[0].pending = Some(vec![]);
        let running = summary(&frames[0], Some(Kind::WorkQueue), "");
        assert_eq!(running.pressured, 1);
        assert_eq!(running.contended, 0);
        assert_eq!(running.with_waiters, 0);
        frames[0].workqs[0].flags = Some(1);
        assert_eq!(summary(&frames[0], Some(Kind::WorkQueue), "").pressured, 0);
        frames[0].workqs[0].pending = Some(vec!["task".into()]);
        assert_eq!(summary(&frames[0], Some(Kind::WorkQueue), "").pressured, 1);
        frames[0].mutexes[0].object.waiters = Some(vec![]);
        let locked = summary(&frames[0], Some(Kind::Mutex), "");
        assert_eq!(locked.pressured, 1);
        assert_eq!(locked.contended, 0);
    }

    #[test]
    fn all_dump_groups_reach_the_overview_with_tui_semantics() {
        let frames = fixture();
        let rows = rows(&frames[0]);
        assert_eq!(rows.len(), 6);
        assert_eq!(rows.iter().map(|r| r.kind).collect::<Vec<_>>(), Kind::ALL);
        let heap = find(&rows, Kind::Heap);
        assert_eq!(heap.summary, "64 / 256 B");
        assert_eq!(heap.fill, Some(25.));
        assert_eq!(heap.waiters, None);
        let slab = find(&rows, Kind::MemorySlab);
        assert_eq!(slab.summary, "2 / 8 × 32 B");
        assert_eq!(slab.fill, Some(25.));
        assert_eq!(slab.peak, Some(75.));
        let queue = find(&rows, Kind::MessageQueue);
        assert_eq!(queue.summary, "4 / 4 × 16 B");
        assert_eq!(queue.state, State::Blocked);
        assert!(queue.waiters.unwrap().is_empty());
        let mutex = find(&rows, Kind::Mutex);
        assert_eq!(mutex.summary, "LOCKED · owner ×2");
        assert_eq!(mutex.state, State::Blocked);
        let sem = find(&rows, Kind::Semaphore);
        assert_eq!(sem.summary, "0 / 3");
        assert_eq!(sem.fill, Some(0.));
        assert_eq!(sem.state, State::Busy);
        assert_eq!(sem.waiters.unwrap(), &["worker".to_string()]);
        let work = find(&rows, Kind::WorkQueue);
        assert_eq!(work.summary, "BUSY · 1+ pending");
        assert_eq!(work.fill, Some(100.));
        // As in the TUI, busy execution alone is not pending-work contention.
        assert_eq!(work.state, State::Busy);
    }

    #[test]
    fn type_and_name_or_address_filters_intersect_without_merging_names() {
        let frames = fixture();
        let rows = rows(&frames[0]);
        assert_eq!(rows.iter().filter(|r| r.matches(None, " GATE ")).count(), 2);
        assert_eq!(
            rows.iter()
                .filter(|r| r.matches(Some(Kind::Mutex), "gate"))
                .count(),
            1
        );
        assert_eq!(
            rows.iter()
                .filter(|r| r.matches(Some(Kind::Mutex), "0X3000"))
                .count(),
            1
        );
        assert_eq!(
            rows.iter()
                .filter(|r| r.matches(Some(Kind::Heap), "gate"))
                .count(),
            0
        );
        assert_eq!(rows.iter().filter(|r| r.matches(None, "")).count(), 6);
    }

    #[test]
    fn missing_groups_and_missing_measurements_remain_compatible_and_unknown() {
        let frame = parse(r#"{"threads":[]}"#).unwrap().remove(0);
        assert!(rows(&frame).is_empty());
        let mut frame: Value =
            serde_json::from_str(include_str!("../test-data/kernel-objects-minimal.json")).unwrap();
        for group in ["semaphores", "mutexes", "msgqs", "mem_slabs", "workqs"] {
            frame[0][group] = json!([{"name": "unknown", "address": 0}]);
        }
        let frames = parse(&frame.to_string()).unwrap();
        for row in rows(&frames[0]).iter().filter(|r| r.kind != Kind::Heap) {
            assert_eq!(row.fill, None);
            assert_eq!(row.peak, None);
            assert_eq!(row.waiters, None);
            assert!(!row.summary.contains("FREE"));
            assert!(!row.summary.contains("NOT STARTED"));
        }
    }

    #[test]
    fn state_boundaries_and_unbounded_semaphores_are_not_false_utilization() {
        let mut frames = fixture();
        let f = &mut frames[0];
        f.semaphores[0].limit = Some(0xffff_ffff);
        f.semaphores[0].count = Some(25);
        assert_eq!(find(&rows(f), Kind::Semaphore).summary, "25 / unlimited");
        assert_eq!(find(&rows(f), Kind::Semaphore).fill, None);
        f.semaphores[0].limit = Some(0);
        assert_eq!(find(&rows(f), Kind::Semaphore).summary, "NOT INITIALIZED");
        f.heaps[0].free_bytes = 0;
        assert_eq!(find(&rows(f), Kind::Heap).state, State::Blocked);
        f.mem_slabs[0].num_used = Some(8);
        assert_eq!(find(&rows(f), Kind::MemorySlab).state, State::Blocked);
        f.mem_slabs[0].num_blocks = Some(0);
        assert_eq!(find(&rows(f), Kind::MemorySlab).fill, None);
        assert_eq!(find(&rows(f), Kind::MemorySlab).peak, None);
        f.mutexes[0].owner_address = Some(0);
        assert_eq!(find(&rows(f), Kind::Mutex).summary, "FREE");
        f.mutexes[0].owner_address = Some(0x9999);
        f.mutexes[0].owner_name = None;
        f.mutexes[0].object.waiters = Some(vec![]);
        assert_eq!(find(&rows(f), Kind::Mutex).summary, "LOCKED · 0x9999 ×2");
        assert_eq!(find(&rows(f), Kind::Mutex).state, State::Busy);
        for flags in [5, 9] {
            f.workqs[0].flags = Some(flags);
            assert_eq!(find(&rows(f), Kind::WorkQueue).state, State::Blocked);
        }
        f.workqs[0].flags = Some(0);
        f.workqs[0].pending = Some(vec![]);
        assert!(
            find(&rows(f), Kind::WorkQueue)
                .summary
                .starts_with("NOT STARTED")
        );
    }

    #[test]
    fn complete_wait_queues_are_rebuilt_from_the_current_frame() {
        let mut frames = fixture();
        frames[0].semaphores[0].object.waiters =
            Some(["a", "b", "c", "d"].map(String::from).to_vec());
        assert_eq!(
            find(&rows(&frames[0]), Kind::Semaphore).waiters.unwrap(),
            &["a", "b", "c", "d"].map(String::from)
        );
        let next = rows(&frames[1]);
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].summary, "2 / 3");
        assert!(next[0].waiters.unwrap().is_empty());
        assert_eq!(next[0].state, State::Normal);
    }
}

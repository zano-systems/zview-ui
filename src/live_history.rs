//! Browser-local chart history, independent of full frames and acquisition rate.
//!
//! Correlated values (e.g. a mutex's depth and owner) share a change log so
//! tooltips stay aligned. Overview metrics and heap-map measurements have their
//! own logs. Repeated observations only advance coverage, never allocate a sample.
use crate::{
    kernel_model::{
        MemorySlab, MemorySlabKey, MessageQueue, MessageQueueKey, Mutex, MutexKey, Semaphore,
        SemaphoreKey, WorkQueue, WorkQueueKey,
    },
    model::{self, ChartPoint, ChartSeries, Frame, Heap, HeapKey, LiveSeries, Thread, ThreadKey},
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    mem::size_of,
};

pub const RETENTION_SECONDS: f64 = 300.;
pub const DEFAULT_WINDOW_SECONDS: f64 = 20.;
const MEMORY_BUDGET: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Thread(ThreadKey),
    Heap(HeapKey),
    Semaphore(SemaphoreKey),
    Mutex(MutexKey),
    MessageQueue(MessageQueueKey),
    Slab(MemorySlabKey),
    WorkQueue(WorkQueueKey),
}

#[derive(Clone, Debug, PartialEq)]
enum Reading {
    Thread(Thread),
    Heap(Heap),
    Semaphore(Semaphore),
    Mutex(Mutex),
    MessageQueue(MessageQueue),
    Slab(MemorySlab),
    WorkQueue(WorkQueue),
}

impl Reading {
    fn key(&self) -> Key {
        match self {
            Self::Thread(t) => Key::Thread(t.key()),
            Self::Heap(h) => Key::Heap(HeapKey {
                address: h.address,
                name: h.name.clone(),
            }),
            Self::Semaphore(s) => Key::Semaphore(SemaphoreKey {
                address: s.object.address,
                name: s.object.name.clone(),
            }),
            Self::Mutex(m) => Key::Mutex(MutexKey {
                address: m.object.address,
                name: m.object.name.clone(),
            }),
            Self::MessageQueue(q) => Key::MessageQueue(MessageQueueKey {
                address: q.object.address,
                name: q.object.name.clone(),
            }),
            Self::Slab(s) => Key::Slab(MemorySlabKey {
                address: s.object.address,
                name: s.object.name.clone(),
            }),
            Self::WorkQueue(q) => Key::WorkQueue(WorkQueueKey {
                address: q.object.address,
                name: q.object.name.clone(),
            }),
        }
    }

    fn extra_bytes(&self) -> usize {
        let strings = |items: &Option<Vec<String>>| {
            items.as_ref().map_or(0, |v| {
                v.capacity() * size_of::<String>() + v.iter().map(String::capacity).sum::<usize>()
            })
        };
        match self {
            Self::Thread(t) => t.name.capacity(),
            Self::Heap(h) => h.name.capacity() + strings(&h.waiters),
            Self::Semaphore(s) => s.object.name.capacity() + strings(&s.object.waiters),
            Self::Mutex(m) => {
                m.object.name.capacity()
                    + strings(&m.object.waiters)
                    + m.owner_name.as_ref().map_or(0, String::capacity)
            }
            Self::MessageQueue(q) => q.object.name.capacity() + strings(&q.object.waiters),
            Self::Slab(s) => s.object.name.capacity() + strings(&s.object.waiters),
            Self::WorkQueue(q) => {
                q.object.name.capacity() + strings(&q.object.waiters) + strings(&q.pending)
            }
        }
    }

    fn put(&self, frame: &mut Frame) {
        match self {
            Self::Thread(t) => frame.threads.push(t.clone()),
            Self::Heap(h) => frame.heaps.push(h.clone()),
            Self::Semaphore(s) => frame.semaphores.push(s.clone()),
            Self::Mutex(m) => frame.mutexes.push(m.clone()),
            Self::MessageQueue(q) => frame.msgqs.push(q.clone()),
            Self::Slab(s) => frame.mem_slabs.push(s.clone()),
            Self::WorkQueue(q) => frame.workqs.push(q.clone()),
        }
    }
}

struct Span<T> {
    at: f64,
    /// Last actual observation in this unchanged run. Keep both ends so
    /// connecting percentage samples cannot move a later rise back in time.
    last: f64,
    until: f64,
    value: T,
    bytes: usize,
}

struct Log<T> {
    spans: VecDeque<Span<T>>,
    continuous: bool,
    bytes: usize,
}

impl<T> Default for Log<T> {
    fn default() -> Self {
        Self {
            spans: VecDeque::new(),
            continuous: false,
            bytes: 0,
        }
    }
}

impl<T: PartialEq> Log<T> {
    fn allocated_bytes(&self) -> usize {
        // Count spare deque capacity conservatively as well as owned payloads.
        self.bytes + self.spans.capacity() * size_of::<Span<T>>()
    }

    fn observe(&mut self, at: f64, value: T, extra_bytes: usize) {
        if self.continuous
            && let Some(last) = self.spans.back_mut()
        {
            if last.value == value {
                last.last = at;
                last.until = at;
                return;
            }
            // Only consecutive successful observations establish a held interval.
            last.until = at;
        }
        let bytes = size_of::<Span<T>>() + extra_bytes;
        self.spans.push_back(Span {
            at,
            last: at,
            until: at,
            value,
            bytes,
        });
        self.bytes += bytes;
        self.continuous = true;
    }

    fn remove_front(&mut self) -> Option<f64> {
        let span = self.spans.pop_front()?;
        self.bytes -= span.bytes;
        if self.spans.capacity() > self.spans.len().max(1) * 2 {
            self.spans.shrink_to_fit();
        }
        Some(span.until)
    }

    fn prune(&mut self, cutoff: f64) {
        // A constant interval may start before the boundary and still cover it.
        while self.spans.front().is_some_and(|s| s.until < cutoff) {
            self.remove_front();
        }
    }
}

pub struct History {
    objects: HashMap<Key, Log<Reading>>,
    fragmentation: HashMap<HeapKey, Log<Option<f64>>>,
    cpu: Log<Option<f64>>,
    stacks: Log<Option<f64>>,
    start: Option<f64>,
    origin: Option<f64>,
    end: f64,
    memory_floor: f64,
    memory_limited: bool,
    budget: usize,
}

impl Default for History {
    fn default() -> Self {
        Self {
            objects: HashMap::new(),
            fragmentation: HashMap::new(),
            cpu: Log::default(),
            stacks: Log::default(),
            start: None,
            origin: None,
            end: 0.,
            memory_floor: 0.,
            memory_limited: false,
            budget: MEMORY_BUDGET,
        }
    }
}

impl History {
    pub fn is_empty(&self) -> bool {
        self.origin().is_none()
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// A transport/read interruption ends confirmation, without making up a value.
    pub fn interrupt(&mut self) {
        for log in self.objects.values_mut() {
            log.continuous = false;
        }
        for log in self.fragmentation.values_mut() {
            log.continuous = false;
        }
        self.cpu.continuous = false;
        self.stacks.continuous = false;
    }

    pub fn record(&mut self, frame: &Frame, overview: bool) {
        if !frame.is_live {
            self.interrupt();
            return;
        }
        let Some(at) = frame.elapsed_seconds.filter(|t| t.is_finite() && *t >= 0.) else {
            return;
        };
        let origin = *self.origin.get_or_insert(at);
        let at = at - origin;
        if self.start.is_some() && at <= self.end {
            return;
        }
        self.start.get_or_insert(at);
        self.end = at;
        let mut rows = Vec::new();
        for t in &frame.threads {
            let mut t = t.clone();
            t.priority = None;
            t.user_options = None;
            t.entry_symbol = None;
            t.entry_file = None;
            t.entry_line = None;
            if let Some(runtime) = &mut t.runtime {
                runtime.active = false;
            }
            rows.push(Reading::Thread(t));
        }
        for h in &frame.heaps {
            if let Some(chunks) = &h.chunks {
                let value = crate::heap_map::ChunkMap::new(chunks).map(|map| map.fragmentation());
                self.fragmentation
                    .entry(HeapKey {
                        address: h.address,
                        name: h.name.clone(),
                    })
                    .or_default()
                    .observe(at, value, 0);
            }
            // Do not clone chunk buffers into the chart projection.
            let h = Heap {
                address: h.address,
                name: h.name.clone(),
                allocated_bytes: h.allocated_bytes,
                free_bytes: h.free_bytes,
                max_allocated_bytes: None,
                chunks: None,
                waiters: counts(&h.waiters),
            };
            rows.push(Reading::Heap(h));
        }
        for s in &frame.semaphores {
            let mut s = s.clone();
            s.object.waiters = counts(&s.object.waiters);
            rows.push(Reading::Semaphore(s));
        }
        for m in &frame.mutexes {
            let mut m = m.clone();
            m.object.waiters = counts(&m.object.waiters);
            rows.push(Reading::Mutex(m));
        }
        for q in &frame.msgqs {
            let mut q = q.clone();
            q.object.waiters = None;
            q.msg_size = None;
            rows.push(Reading::MessageQueue(q));
        }
        for s in &frame.mem_slabs {
            let mut s = s.clone();
            s.object.waiters = None;
            s.max_used = None;
            s.block_size = None;
            rows.push(Reading::Slab(s));
        }
        for q in &frame.workqs {
            let mut q = q.clone();
            q.pending = counts(&q.pending);
            q.object.waiters = None;
            q.thread_name = None;
            rows.push(Reading::WorkQueue(q));
        }
        let mut seen = HashSet::new();
        for row in rows {
            let key = row.key();
            let extra = row.extra_bytes();
            seen.insert(key.clone());
            self.objects.entry(key).or_default().observe(at, row, extra);
        }
        for (key, log) in &mut self.objects {
            if !seen.contains(key) {
                log.continuous = false;
            }
        }
        for (key, log) in &mut self.fragmentation {
            if !seen.contains(&Key::Heap(key.clone())) {
                log.continuous = false;
            }
        }
        if overview {
            self.cpu
                .observe(at, frame.cpu().filter(|v| v.is_finite()), 0);
            self.stacks.observe(
                at,
                frame
                    .stacks()
                    .and_then(|(used, size)| model::percentage(used, size)),
                0,
            );
        } else {
            self.cpu.continuous = false;
            self.stacks.continuous = false;
        }
        self.prune();
        self.enforce_budget();
    }

    fn cutoff(&self) -> f64 {
        (self.end - RETENTION_SECONDS)
            .max(self.start.unwrap_or(0.))
            .max(self.memory_floor)
    }

    fn prune(&mut self) {
        let cutoff = self.cutoff();
        self.objects.retain(|_, log| {
            log.prune(cutoff);
            !log.spans.is_empty()
        });
        self.fragmentation.retain(|_, log| {
            log.prune(cutoff);
            !log.spans.is_empty()
        });
        self.cpu.prune(cutoff);
        self.stacks.prune(cutoff);
        if self.objects.capacity() > self.objects.len().max(1) * 2 {
            self.objects.shrink_to_fit();
        }
        if self.fragmentation.capacity() > self.fragmentation.len().max(1) * 2 {
            self.fragmentation.shrink_to_fit();
        }
    }

    pub fn bytes(&self) -> usize {
        let name_bytes = |key: &Key| match key {
            Key::Thread(k) => k.name.capacity(),
            Key::Heap(k) => k.name.capacity(),
            Key::Semaphore(k) => k.name.capacity(),
            Key::Mutex(k) => k.name.capacity(),
            Key::MessageQueue(k) => k.name.capacity(),
            Key::Slab(k) => k.name.capacity(),
            Key::WorkQueue(k) => k.name.capacity(),
        };
        self.objects.capacity() * size_of::<(Key, Log<Reading>)>()
            + self.fragmentation.capacity() * size_of::<(HeapKey, Log<Option<f64>>)>()
            + self
                .objects
                .iter()
                .map(|(key, log)| log.allocated_bytes() + name_bytes(key) + 256)
                .sum::<usize>()
            + self
                .fragmentation
                .iter()
                .map(|(key, log)| log.allocated_bytes() + key.name.capacity() + 128)
                .sum::<usize>()
            + self.cpu.allocated_bytes()
            + self.stacks.allocated_bytes()
    }

    pub fn changes(&self) -> usize {
        self.objects
            .values()
            .map(|log| log.spans.len())
            .sum::<usize>()
            + self
                .fragmentation
                .values()
                .map(|log| log.spans.len())
                .sum::<usize>()
            + self.cpu.spans.len()
            + self.stacks.spans.len()
    }

    fn enforce_budget(&mut self) {
        while self.bytes() > self.budget {
            // Evict by oldest coverage, not sample count. Report the resulting
            // reduced time horizon; never average away spikes in stored history.
            let object = self
                .objects
                .iter()
                .filter_map(|(key, log)| Some((key.clone(), log.spans.front()?.until)))
                .min_by(|a, b| a.1.total_cmp(&b.1));
            let fragment = self
                .fragmentation
                .iter()
                .filter_map(|(key, log)| Some((key.clone(), log.spans.front()?.until)))
                .min_by(|a, b| a.1.total_cmp(&b.1));
            let oldest = [
                object.as_ref().map(|(_, t)| *t),
                fragment.as_ref().map(|(_, t)| *t),
                self.cpu.spans.front().map(|s| s.until),
                self.stacks.spans.front().map(|s| s.until),
            ];
            let Some((which, until)) = oldest
                .into_iter()
                .enumerate()
                .filter_map(|(i, t)| t.map(|t| (i, t)))
                .min_by(|a, b| a.1.total_cmp(&b.1))
            else {
                break;
            };
            match which {
                0 => {
                    self.objects
                        .get_mut(&object.unwrap().0)
                        .unwrap()
                        .remove_front();
                }
                1 => {
                    self.fragmentation
                        .get_mut(&fragment.unwrap().0)
                        .unwrap()
                        .remove_front();
                }
                2 => {
                    self.cpu.remove_front();
                }
                _ => {
                    self.stacks.remove_front();
                }
            }
            self.memory_floor = self.memory_floor.max(until);
            self.memory_limited = true;
            self.prune();
        }
    }

    fn window(&self) -> (f64, f64) {
        let start = self.cutoff();
        (
            start,
            self.end
                .max(start + f64::EPSILON * self.end.abs().max(1.) * 16.),
        )
    }

    pub fn limited(&self) -> bool {
        self.memory_limited
            && self.memory_floor >= (self.end - RETENTION_SECONDS).max(self.start.unwrap_or(0.))
    }

    pub fn label(&self) -> String {
        if self.is_empty() {
            return "View history · waiting for samples since opening this screen".into();
        }
        format!(
            "View history · {:.1}s retained · {} changes · {:.0} KiB{}",
            self.retained_seconds(),
            self.changes(),
            self.bytes() as f64 / 1024.,
            if self.limited() {
                " · memory limit shortened history"
            } else {
                ""
            }
        )
    }

    pub fn retained_seconds(&self) -> f64 {
        self.end - self.cutoff()
    }

    pub fn footer_label(&self) -> String {
        if self.is_empty() {
            return "History · waiting for readings".into();
        }
        format!(
            "History · {:.1}s · {} changes · {:.0} KiB{}",
            self.retained_seconds(),
            self.changes(),
            self.bytes() as f64 / 1024.,
            if self.limited() {
                " · memory limited"
            } else {
                ""
            }
        )
    }

    pub fn origin(&self) -> Option<f64> {
        self.origin
    }

    pub fn frames(&self, key: &Key) -> Vec<Frame> {
        let mut result = Vec::new();
        if let Some(log) = self.objects.get(key) {
            for span in &log.spans {
                let mut frame = self.empty_frame(
                    span.at,
                    if span.last > span.at {
                        span.last
                    } else {
                        span.until
                    },
                );
                span.value.put(&mut frame);
                result.push(frame);
                if span.last > span.at {
                    let mut frame = self.empty_frame(span.last, span.until);
                    span.value.put(&mut frame);
                    result.push(frame);
                }
            }
        }
        if result.is_empty() {
            result.push(self.empty_frame(self.end, self.end));
        }
        result
    }

    fn empty_frame(&self, at: f64, until: f64) -> Frame {
        Frame {
            elapsed_seconds: Some(at),
            is_live: true,
            observation_end: Some(until),
            history_window: Some(self.window()),
            history_limited: self.limited(),
            ..Frame::default()
        }
    }

    fn series(&self, log: Option<&Log<Option<f64>>>) -> ChartSeries {
        let mut points = Vec::new();
        let mut ends = Vec::new();
        if let Some(log) = log {
            for span in &log.spans {
                points.push(ChartPoint {
                    x: span.at,
                    value: span.value,
                });
                ends.push(if span.last > span.at {
                    span.last
                } else {
                    span.until
                });
                if span.last > span.at {
                    points.push(ChartPoint {
                        x: span.last,
                        value: span.value,
                    });
                    ends.push(span.until);
                }
            }
        }
        ChartSeries {
            points,
            x_domain: self.window(),
            timed: true,
            playhead: None,
            caption: self.label(),
            live: Some(LiveSeries {
                ends,
                limited: self.limited(),
            }),
        }
    }

    pub fn cpu(&self) -> ChartSeries {
        self.series(Some(&self.cpu))
    }
    pub fn stacks(&self) -> ChartSeries {
        self.series(Some(&self.stacks))
    }
    pub fn unknown(&self) -> ChartSeries {
        self.series(None)
    }
    pub fn fragmentation(&self, key: &HeapKey) -> ChartSeries {
        self.series(self.fragmentation.get(key))
    }
}

fn counts(items: &Option<Vec<String>>) -> Option<Vec<String>> {
    items.as_ref().map(|v| vec![String::new(); v.len()])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(at: f64, cpu: f64) -> Frame {
        let mut f = model::parse(include_str!("../sample.json"))
            .unwrap()
            .remove(0);
        f.elapsed_seconds = Some(at);
        f.is_live = true;
        f.threads.truncate(1);
        f.threads[0].runtime.as_mut().unwrap().cpu_normalized = Some(cpu);
        f
    }

    #[test]
    fn duration_not_sample_count_and_repeated_values_only_extend_coverage() {
        for hz in [1, 10, 100] {
            let mut h = History::default();
            for i in 0..=400 * hz {
                h.record(&frame(i as f64 / hz as f64, 25.), true);
            }
            let series = h.cpu();
            assert_eq!(h.cpu.spans.len(), 1);
            assert_eq!(series.points.len(), 2);
            assert_eq!(series.x_domain, (100., 400.));
            assert_eq!(series.interval_end(0), Some(400.));
            assert_eq!(series.held_index(250.), Some(0));
            assert!(h.changes() < 10);
        }
    }

    #[test]
    fn changes_spikes_boundary_seed_and_explicit_gaps_are_preserved() {
        let mut h = History::default();
        for (t, cpu) in [(0., 1.), (100., 1.), (101., 99.), (101.01, 1.), (350., 1.)] {
            h.record(&frame(t, cpu), true);
        }
        let series = h.cpu();
        assert_eq!(series.points.len(), 5);
        assert_eq!(series.points[0].x, 0.);
        assert_eq!(series.held_index(50.), Some(0));
        assert_eq!(
            series.points[series.held_index(101.).unwrap()].value,
            Some(99.)
        );
        h.interrupt();
        h.record(&frame(370., 1.), true);
        let series = h.cpu();
        assert_eq!(series.held_index(360.), None);
        assert_eq!(
            series.points[series.held_index(370.).unwrap()].value,
            Some(1.)
        );
        assert_eq!(
            series.interval_end(series.held_index(350.).unwrap()),
            Some(350.)
        );
    }

    #[test]
    fn unobserved_intervals_remain_gaps_without_inventing_values() {
        let mut h = History::default();
        let first = frame(0., 10.);
        let key = Key::Thread(first.threads[0].key());
        h.record(&first, true);
        h.record(&frame(10., 10.), true);
        let mut other = frame(20., 10.);
        other.threads.clear();
        h.record(&other, false);
        h.record(&frame(30., 10.), true);
        let frames = h.frames(&key);
        let series = model::chart_series(&frames, frames.len() - 1, |f| f.cpu());
        assert_eq!(series.held_index(5.), Some(0));
        assert_eq!(series.held_index(15.), None);
        assert!(series.held_index(30.).is_some());
    }

    #[test]
    fn chart_projections_do_not_retain_maps_or_unplotted_metadata() {
        let mut h = History::default();
        let mut f = frame(0., 10.);
        f.heaps[0].chunks = Some(vec![
            model::HeapChunk {
                size: 32,
                used: false
            };
            1000
        ]);
        let key = Key::Heap(HeapKey {
            address: f.heaps[0].address,
            name: f.heaps[0].name.clone(),
        });
        h.record(&f, true);
        let changes = h.changes();
        f.elapsed_seconds = Some(1.);
        f.threads[0].priority = Some(999);
        f.threads[0].runtime.as_mut().unwrap().active = true;
        h.record(&f, true);
        assert_eq!(h.changes(), changes);
        assert!(h.frames(&key).iter().all(|f| f.heaps[0].chunks.is_none()));
        assert_eq!(h.fragmentation.len(), 1);
    }

    #[test]
    fn each_screen_visit_starts_at_zero_without_reusing_old_history() {
        let mut h = History::default();
        h.record(&frame(239., 10.), true);
        h.record(&frame(245., 20.), true);
        assert_eq!(h.origin(), Some(239.));
        assert_eq!(h.retained_seconds(), 6.);
        h.clear();
        h.record(&frame(262., 97.), true);
        h.record(&frame(263., 98.), true);
        let series = h.cpu();
        assert_eq!(h.origin(), Some(262.));
        assert_eq!(h.retained_seconds(), 1.);
        assert_eq!(series.x_domain, (0., 1.));
        assert_eq!(series.points[0].x, 0.);
        assert!(
            series
                .points
                .iter()
                .all(|point| point.value.unwrap() >= 97.)
        );
    }

    #[test]
    fn first_and_last_equal_observations_preserve_the_shape_before_a_rise() {
        let mut h = History::default();
        for (at, cpu) in [(100., 0.), (101., 0.), (102., 80.), (103., 80.)] {
            h.record(&frame(at, cpu), true);
        }
        assert_eq!(h.cpu.spans.len(), 2);
        let mut series = h.cpu();
        series.compact_live();
        assert_eq!(
            series.points,
            vec![
                ChartPoint {
                    x: 0.,
                    value: Some(0.)
                },
                ChartPoint {
                    x: 1.,
                    value: Some(0.)
                },
                ChartPoint {
                    x: 2.,
                    value: Some(80.)
                },
                ChartPoint {
                    x: 3.,
                    value: Some(80.)
                },
            ]
        );
    }

    #[test]
    fn memory_guard_shortens_history_explicitly_without_averaging() {
        let mut h = History {
            budget: 4096,
            ..History::default()
        };
        for i in 0..1000 {
            h.record(&frame(i as f64 * 0.01, (i % 2) as f64), true);
        }
        assert!(h.bytes() <= h.budget);
        assert!(h.limited());
        assert!(h.label().contains("memory limit"));
        assert!(
            h.cpu()
                .points
                .iter()
                .all(|p| p.value == Some(0.) || p.value == Some(1.))
        );
    }

    #[test]
    fn dense_rendering_keeps_spikes_and_hover_uses_exact_retained_changes() {
        let mut h = History::default();
        for i in 0..1000 {
            let value = if i == 501 { 99. } else { (i % 2) as f64 };
            h.record(&frame(i as f64 / 100., value), true);
        }
        let series = h.cpu();
        let envelopes = series.envelopes(20).unwrap();
        assert!(envelopes.len() <= 20);
        assert_eq!(
            envelopes
                .iter()
                .flatten()
                .map(|b| b.maximum)
                .fold(0., f64::max),
            99.
        );
        let exact = series.held_index(5.01).unwrap();
        assert_eq!(series.points[exact].value, Some(99.));
        let mut zoomed = series;
        zoomed.x_domain = (5., 5.02);
        assert!(zoomed.envelopes(20).is_none());
    }

    #[test]
    fn unknown_series_has_the_same_time_axis_and_dense_gaps_are_not_held_values() {
        let mut h = History::default();
        for i in 0..500 {
            h.record(&frame(i as f64 / 100., (i % 2) as f64), true);
        }
        h.interrupt();
        h.record(&frame(6., 10.), true);
        let series = h.cpu();
        assert_eq!(series.held_index(5.5), None);
        assert_eq!(h.unknown().x_domain, series.x_domain);
        assert!(h.unknown().points.is_empty());
        assert!(
            series
                .envelopes(1)
                .unwrap()
                .iter()
                .flatten()
                .any(|b| b.has_gap)
        );
    }

    #[test]
    fn per_object_histories_preserve_identity_and_correlated_owner_changes() {
        let mut h = History::default();
        let mut f = model::parse(include_str!("../test-data/mutexes.json"))
            .unwrap()
            .remove(0);
        f.is_live = true;
        f.elapsed_seconds = Some(0.);
        let m = &mut f.mutexes[0];
        m.lock_count = Some(1);
        m.owner_address = Some(100);
        let key = Key::Mutex(MutexKey {
            address: m.object.address,
            name: m.object.name.clone(),
        });
        h.record(&f, false);
        f.elapsed_seconds = Some(1.);
        f.mutexes[0].owner_address = Some(200);
        h.record(&f, false);
        let snapshots = h.frames(&key);
        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[0].mutexes[0].owner_address, Some(100));
        assert_eq!(snapshots[1].mutexes[0].owner_address, Some(200));
        assert!(
            snapshots
                .iter()
                .all(|f| f.threads.is_empty() && f.heaps.is_empty())
        );
    }
}

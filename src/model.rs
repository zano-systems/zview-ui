use serde::Deserialize;
use std::{fs, path::Path};

#[derive(Clone, Debug, Deserialize)]
pub struct Frame {
    pub threads: Vec<Thread>,
    #[serde(default)]
    pub heaps: Vec<Heap>,
    #[serde(default)]
    pub semaphores: Vec<crate::kernel_model::Semaphore>,
    #[serde(default)]
    pub mutexes: Vec<crate::kernel_model::Mutex>,
    #[serde(default)]
    pub msgqs: Vec<crate::kernel_model::MessageQueue>,
    #[serde(default)]
    pub mem_slabs: Vec<crate::kernel_model::MemorySlab>,
    #[serde(default)]
    pub workqs: Vec<crate::kernel_model::WorkQueue>,
    /// Seconds since the first usable frame; absent in ordinary single-frame dumps.
    pub elapsed_seconds: Option<f64>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Thread {
    pub address: u64,
    pub name: String,
    pub stack_start: Option<u64>,
    pub stack_size: Option<u64>,
    pub runtime: Option<Runtime>,
    pub priority: Option<i32>,
    pub user_options: Option<u64>,
    pub entry_point: Option<u64>,
    pub entry_symbol: Option<String>,
    pub entry_file: Option<String>,
    pub entry_line: Option<u64>,
}

impl Thread {
    pub fn activity_description(&self) -> &'static str {
        match self.runtime.as_ref() {
            Some(runtime) if runtime.active => {
                "CPU activity observed since the previous sample; not current scheduler state"
            }
            Some(runtime) if runtime.cpu.is_some() || runtime.cpu_normalized.is_some() => {
                "No CPU activity observed since the previous sample; scheduler state unknown"
            }
            _ => "CPU activity unavailable; scheduler state unknown",
        }
    }

    pub fn entry_source(&self) -> String {
        match self.entry_file.as_deref().filter(|path| !path.is_empty()) {
            Some(path) => {
                // Match the TUI's widest SiteValue form: the final three path
                // components. Keep entry_file untouched for the full location.
                let parts: Vec<_> = path
                    .split(['/', '\\'])
                    .filter(|part| !part.is_empty())
                    .collect();
                let path = parts[parts.len().saturating_sub(3)..].join("/");
                match self.entry_line.filter(|line| *line > 0) {
                    Some(line) => format!("{path}:{line}"),
                    None => path,
                }
            }
            None => "—".into(),
        }
    }

    pub fn entry_description(&self) -> String {
        let source = self.entry_source();
        let symbol = self
            .entry_symbol
            .as_deref()
            .filter(|symbol| !symbol.is_empty());
        let label = match (symbol, source.as_str()) {
            (Some(symbol), "—") => Some(symbol.to_string()),
            (Some(symbol), source) => Some(format!("{symbol} @ {source}")),
            (None, "—") => None,
            (None, source) => Some(source.to_string()),
        };
        match (label, self.entry_point) {
            (Some(label), Some(address)) => format!("{label} (0x{address:08x})"),
            (Some(label), None) => label,
            (None, Some(address)) => format!("0x{address:08x}"),
            (None, None) => "—".into(),
        }
    }
}

/// Do not join histories by display name, or across a known reuse of an address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadKey {
    pub address: u64,
    pub name: String,
    stack_start: Option<u64>,
    entry_point: Option<u64>,
}

impl Thread {
    pub fn key(&self) -> ThreadKey {
        ThreadKey {
            address: self.address,
            name: self.name.clone(),
            stack_start: self.stack_start,
            entry_point: self.entry_point,
        }
    }
}

impl ThreadKey {
    pub fn in_frame<'a>(&self, frame: &'a Frame) -> Option<&'a Thread> {
        frame.threads.iter().find(|thread| {
            thread.address == self.address
                && thread.name == self.name
                && thread.stack_start == self.stack_start
                && thread.entry_point == self.entry_point
        })
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Runtime {
    pub cpu: Option<f64>,
    pub cpu_normalized: Option<f64>,
    pub active: bool,
    pub stack_watermark: Option<u64>,
    pub stack_watermark_percent: Option<f64>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Heap {
    pub address: u64,
    pub name: String,
    pub allocated_bytes: u64,
    pub free_bytes: u64,
    pub max_allocated_bytes: Option<u64>,
    pub chunks: Option<Vec<HeapChunk>>,
    pub waiters: Option<Vec<String>>,
}

/// The dump supplies address-ordered sizes, not absolute chunk addresses.
#[derive(Clone, Debug, Deserialize)]
pub struct HeapChunk {
    pub size: u64,
    pub used: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeapKey {
    pub address: u64,
    pub name: String,
}

impl HeapKey {
    pub fn in_frame<'a>(&self, frame: &'a Frame) -> Option<&'a Heap> {
        frame
            .heaps
            .iter()
            .find(|heap| heap.address == self.address && heap.name == self.name)
    }
}

impl Heap {
    pub fn label(&self) -> String {
        format!("{} · {:#x}", self.name, self.address)
    }

    pub fn total(&self) -> u64 {
        self.allocated_bytes.saturating_add(self.free_bytes)
    }

    pub fn percent(&self) -> Option<f64> {
        percentage(self.allocated_bytes, self.total())
    }
}

impl Frame {
    pub fn visible_threads(&self) -> Vec<&Thread> {
        let mut threads: Vec<_> = self.threads.iter().filter(|t| t.name != "idle").collect();
        threads.sort_by(|a, b| {
            let cpu = |t: &Thread| t.runtime.as_ref().and_then(|r| r.cpu_normalized);
            cpu(b)
                .partial_cmp(&cpu(a))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| a.address.cmp(&b.address))
        });
        threads
    }

    pub fn cpu(&self) -> Option<f64> {
        let threads = self.visible_threads();
        if threads.is_empty() {
            return None;
        }
        threads
            .iter()
            .map(|t| t.runtime.as_ref()?.cpu_normalized)
            .sum()
    }

    pub fn stacks(&self) -> Option<(u64, u64)> {
        let threads = self.visible_threads();
        if threads.is_empty() {
            return None;
        }
        threads.iter().try_fold((0_u64, 0_u64), |(used, size), t| {
            Some((
                used.checked_add(t.runtime.as_ref()?.stack_watermark?)?,
                size.checked_add(t.stack_size?)?,
            ))
        })
    }
}

pub fn percentage(used: u64, size: u64) -> Option<f64> {
    (size > 0).then(|| used as f64 / size as f64 * 100.)
}

pub fn percent(value: Option<f64>) -> String {
    value
        .map(|v| format!("{v:.1}%"))
        .unwrap_or_else(|| "—".into())
}

/// Choose a recorded frame at or before the requested time, without reading ahead.
pub fn frame_at(frames: &[Frame], seconds: f64) -> usize {
    frames
        .partition_point(|f| f.elapsed_seconds.is_some_and(|t| t <= seconds))
        .saturating_sub(1)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChartPoint {
    pub x: f64,
    pub value: Option<f64>,
}

pub struct ChartSeries {
    /// One entry per source frame through the selected frame, including gaps.
    pub points: Vec<ChartPoint>,
    pub x_domain: (f64, f64),
    pub timed: bool,
    pub caption: String,
    /// View-only playback cursor; never a captured sample or a new reading.
    pub playhead: Option<f64>,
}

impl ChartSeries {
    /// Include the predecessor so a held interval entering the left edge remains
    /// visible. Keep original indices for metadata and source-reading tooltips.
    pub fn visible_range(&self) -> std::ops::Range<usize> {
        let first = self
            .points
            .partition_point(|p| p.x < self.x_domain.0)
            .saturating_sub(1);
        let end = self.points.partition_point(|p| p.x <= self.x_domain.1);
        first.min(end)..end
    }

    pub fn with_playhead(mut self, playhead: Option<f64>) -> Self {
        self.playhead = playhead;
        self
    }

    pub fn view_end(&self) -> Option<f64> {
        self.playhead.or_else(|| self.points.last().map(|p| p.x))
    }

    pub fn interval_end(&self, index: usize) -> Option<f64> {
        let sample = self.points.get(index)?;
        self.points
            .get(index + 1)
            .map(|p| p.x)
            .or_else(|| self.playhead.filter(|end| *end > sample.x))
    }

    pub fn segments(&self) -> impl Iterator<Item = &[ChartPoint]> {
        self.points
            .split(|point| point.value.is_none())
            .filter(|segment| !segment.is_empty())
    }

    /// Hold the left source reading until the next timestamp, including up to
    /// a missing reading. Never extend beyond the selected frame.
    pub fn held_index(&self, x: f64) -> Option<usize> {
        if !x.is_finite() {
            return None;
        }
        let right = self.points.partition_point(|point| point.x < x);
        if let Some(point) = self.points.get(right)
            && point.x == x
        {
            return point.value.map(|_| right);
        }
        let left = right.checked_sub(1)?;
        if self.points.get(right).is_none() && self.playhead.is_none_or(|end| x > end) {
            return None;
        }
        self.points[left].value.map(|_| left)
    }

    pub fn source_label(&self, x: f64) -> String {
        if self.timed {
            format!("{x}s")
        } else {
            format!("Frame {x:.0}")
        }
    }
}

/// Keep original timestamps and values: no uniform-grid resampling or synthetic
/// hold-last-value points. The full document's x-domain remains stable as the
/// selected frame changes, with future readings omitted and gaps retained.
pub fn chart_series(
    frames: &[Frame],
    current: usize,
    value: impl Fn(&Frame) -> Option<f64>,
) -> ChartSeries {
    let timed = frames[current].elapsed_seconds.is_some();
    let x_domain = if timed {
        (
            0.,
            frames
                .last()
                .and_then(|f| f.elapsed_seconds)
                .unwrap()
                .ceil()
                .max(1.),
        )
    } else {
        (1., (frames.len() as f64).max(2.))
    };
    let points = frames[..=current]
        .iter()
        .enumerate()
        .map(|(index, frame)| ChartPoint {
            x: frame.elapsed_seconds.unwrap_or((index + 1) as f64),
            value: value(frame).filter(|value| value.is_finite()),
        })
        .collect();
    ChartSeries {
        points,
        x_domain,
        timed,
        playhead: None,
        caption: if let Some(now) = frames[current].elapsed_seconds {
            format!("{now:.2}s / {:.0}s · recorded time", x_domain.1)
        } else if frames.len() == 1 {
            "Single snapshot · no history".into()
        } else {
            format!("Capture order · through frame {}", current + 1)
        },
    }
}

/// Percentage history for the heap selected in the overview.
pub fn heap_usage_series(frames: &[Frame], current: usize, key: Option<&HeapKey>) -> ChartSeries {
    chart_series(frames, current, |frame| key?.in_frame(frame)?.percent())
}

pub fn parse(input: &str) -> Result<Vec<Frame>, String> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Document {
        Single(Frame),
        Series(Vec<Frame>),
    }
    let frames = match serde_json::from_str(input).map_err(|e| format!("Invalid dump JSON: {e}"))? {
        Document::Single(frame) => vec![frame],
        Document::Series(frames) => frames,
    };
    if frames.is_empty() {
        return Err("The snapshot array is empty.".into());
    }
    let timed = frames
        .iter()
        .filter(|f| f.elapsed_seconds.is_some())
        .count();
    if timed != 0
        && (timed != frames.len()
            || frames.iter().any(|f| {
                !f.elapsed_seconds.unwrap().is_finite() || f.elapsed_seconds.unwrap() < 0.
            })
            || frames[0].elapsed_seconds != Some(0.)
            || frames
                .windows(2)
                .any(|w| w[0].elapsed_seconds >= w[1].elapsed_seconds))
    {
        return Err(
            "Timed snapshots must all have finite, increasing elapsed_seconds starting at zero."
                .into(),
        );
    }
    Ok(frames)
}

pub fn load(path: &Path) -> Result<Vec<Frame>, String> {
    use std::io::Read;
    const LIMIT: u64 = 16 * 1024 * 1024;
    let file = fs::File::open(path).map_err(|e| format!("Cannot open {}: {e}", path.display()))?;
    let mut text = String::new();
    file.take(LIMIT + 1)
        .read_to_string(&mut text)
        .map_err(|e| format!("Cannot read dump: {e}"))?;
    if text.len() as u64 > LIMIT {
        return Err("Dump exceeds the 16 MiB limit.".into());
    }
    parse(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Frame {
        parse(include_str!("../sample.json")).unwrap().remove(0)
    }

    #[test]
    fn matches_real_dump_and_excludes_idle() {
        let f = sample();
        assert_eq!(f.visible_threads().len(), 2);
        assert_eq!(f.visible_threads()[0].name, "stress_id");
        assert!((f.cpu().unwrap() - 0.121993447).abs() < 0.00001);
        assert_eq!(f.stacks(), Some((984, 2048)));
        assert_eq!(f.heaps[0].percent(), Some(75.));
    }

    #[test]
    fn missing_measurements_are_not_zero() {
        let mut f = sample();
        f.threads[0].runtime = None;
        assert_eq!(f.cpu(), None);
        assert_eq!(f.stacks(), None);
        assert_eq!(percent(None), "—");
        assert_eq!(percentage(0, 0), None);
    }

    #[test]
    fn heap_usage_series_keeps_reused_addresses_and_duplicate_names_separate() {
        let frames = parse(include_str!("../test-data/heaps.json")).unwrap();
        let original = HeapKey {
            address: 4096,
            name: "packet_heap".into(),
        };
        let series = heap_usage_series(&frames, 5, Some(&original));
        assert_eq!(
            series.points.iter().map(|p| p.value).collect::<Vec<_>>(),
            vec![Some(25.), Some(100.), Some(50.), None, Some(0.), Some(50.)],
        );
        assert_eq!(series.points[3].x, 5.);
        let replacement = HeapKey {
            address: 4096,
            name: "replacement_heap".into(),
        };
        let series = heap_usage_series(&frames, 3, Some(&replacement));
        assert!(series.points[..3].iter().all(|p| p.value.is_none()));
        assert_eq!(series.points[3].value, Some(100. * 900. / 1024.));
        let duplicate = HeapKey {
            address: 8192,
            name: "packet_heap".into(),
        };
        let series = heap_usage_series(&frames, 5, Some(&duplicate));
        assert_eq!(series.points[0].value, Some(25.));
        assert!(series.points[1..].iter().all(|p| p.value.is_none()));
        assert!(
            heap_usage_series(&frames, 5, None)
                .points
                .iter()
                .all(|p| p.value.is_none())
        );
    }

    #[test]
    fn accepts_series_but_rejects_invalid_documents() {
        let input = include_str!("../sample.json");
        assert_eq!(parse(&format!("[{input},{input}]")).unwrap().len(), 2);
        assert!(parse("[]").is_err());
        assert!(parse("{}").is_err());
        assert!(parse("not json").is_err());
    }

    fn timed_frames() -> Vec<Frame> {
        [0., 3., 7., 10., 14.9]
            .into_iter()
            .map(|t| {
                let mut frame = sample();
                frame.elapsed_seconds = Some(t);
                frame
            })
            .collect()
    }

    #[test]
    fn timed_view_never_selects_future_data() {
        let frames = timed_frames();
        assert_eq!(frame_at(&frames, 6.9), 1);
        assert_eq!(frame_at(&frames, 7.), 2);
        assert_eq!(frame_at(&frames, 50.), 4);
        let chart = chart_series(&frames, 2, |f| f.elapsed_seconds);
        assert_eq!(chart.x_domain, (0., 15.));
        assert_eq!(
            chart.points,
            vec![
                ChartPoint {
                    x: 0.,
                    value: Some(0.)
                },
                ChartPoint {
                    x: 3.,
                    value: Some(3.)
                },
                ChartPoint {
                    x: 7.,
                    value: Some(7.)
                },
            ]
        );
        assert_eq!(
            chart_series(&frames, 4, |f| f.elapsed_seconds).x_domain,
            chart.x_domain
        );
        assert_eq!(chart.held_index(3.1), Some(1));
        assert_eq!(chart.held_index(6.), Some(1));
        assert_eq!(chart.held_index(7.1), None);
        assert_eq!(chart.source_label(chart.points[1].x), "3s");
    }

    #[test]
    fn history_does_not_connect_across_unknown_values() {
        let frames = timed_frames();
        let chart = chart_series(&frames, 3, |f| f.elapsed_seconds.filter(|t| *t != 7.));
        assert_eq!(chart.points[2], ChartPoint { x: 7., value: None });
        let segments: Vec<_> = chart.segments().collect();
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].len(), 2);
        assert_eq!(
            segments[1],
            &[ChartPoint {
                x: 10.,
                value: Some(10.)
            }]
        );
        assert_eq!(chart.held_index(5.), Some(1));
        assert_eq!(chart.held_index(7.), None);
        assert_eq!(chart.held_index(9.), None);
        assert_eq!(chart.held_index(10.), Some(3));
        let mut zoomed = chart;
        zoomed.x_domain = (8., 10.);
        assert_eq!(zoomed.visible_range(), 2..4);
        assert_eq!(zoomed.points[2].value, None);
        assert_eq!(zoomed.interval_end(2), Some(10.));
        assert_eq!(zoomed.held_index(9.), None);
        zoomed.x_domain = (4., 6.);
        assert_eq!(zoomed.visible_range(), 1..2);
        assert_eq!(zoomed.held_index(5.), Some(1));
    }

    #[test]
    fn capture_order_and_single_points_do_not_fabricate_history() {
        let mut frames = timed_frames();
        for frame in &mut frames {
            frame.elapsed_seconds = None;
        }
        let chart = chart_series(&frames, 2, |_| Some(25.));
        assert_eq!(chart.x_domain, (1., 5.));
        assert_eq!(
            chart.points.iter().map(|p| p.x).collect::<Vec<_>>(),
            vec![1., 2., 3.]
        );
        assert_eq!(chart.source_label(2.), "Frame 2");
        assert_eq!(chart.held_index(4.), None);
        let single = chart_series(&frames[..1], 0, |_| Some(25.));
        assert_eq!(single.points.len(), 1);
        assert_eq!(single.segments().count(), 1);
        assert_eq!(single.caption, "Single snapshot · no history");
        let unknown = chart_series(&frames, 4, |_| Some(f64::NAN));
        assert_eq!(unknown.segments().count(), 0);
        assert_eq!(unknown.held_index(2.), None);
    }

    #[test]
    fn sub_grid_samples_and_precise_timestamps_are_preserved() {
        let mut frames = timed_frames();
        for (frame, time) in frames
            .iter_mut()
            .zip([0., 0.00001, 0.123456789, 3.78, 14.9])
        {
            frame.elapsed_seconds = Some(time);
        }
        let chart = chart_series(&frames, 4, |frame| frame.elapsed_seconds);
        assert_eq!(chart.points.len(), frames.len());
        assert_eq!(chart.points[1].value, Some(0.00001));
        assert_eq!(chart.source_label(chart.points[2].x), "0.123456789s");
        assert_eq!(chart.points[3].x, 3.78);
        assert_eq!(chart.held_index(0.000008), Some(0));
    }

    #[test]
    fn rejects_partial_or_unordered_timestamps() {
        let input = include_str!("../sample.json");
        let mut frame: serde_json::Value = serde_json::from_str(input).unwrap();
        frame["elapsed_seconds"] = 0.0.into();
        assert!(parse(&format!("[{frame},{input}]")).is_err());
        assert!(parse(&format!("[{frame},{frame}]")).is_err());
        frame["elapsed_seconds"] = (-1.0).into();
        assert!(parse(&frame.to_string()).is_err());
    }

    #[test]
    fn metadata_is_optional_but_zero_and_negative_values_are_preserved() {
        let old = sample();
        assert!(old.threads[0].priority.is_none());
        assert!(old.threads[0].entry_symbol.is_none());
        let mut json: serde_json::Value =
            serde_json::from_str(include_str!("../sample.json")).unwrap();
        let thread = &mut json["threads"][0];
        thread["priority"] = (-1).into();
        thread["state"] = 0.into();
        thread["user_options"] = 0.into();
        thread["stack_start"] = 4096.into();
        thread["entry_point"] = 8192.into();
        thread["entry_symbol"] = "worker_main".into();
        thread["entry_file"] = "/workspace/project/src/worker.c".into();
        thread["entry_line"] = 42.into();
        let frames = parse(&json.to_string()).unwrap();
        let thread = &frames[0].threads[0];
        assert_eq!(thread.priority, Some(-1));
        assert_eq!(thread.user_options, Some(0));
        assert_eq!(thread.stack_start, Some(4096));
        assert_eq!(thread.entry_point, Some(8192));
        assert_eq!(thread.entry_symbol.as_deref(), Some("worker_main"));
        assert_eq!(thread.entry_source(), "project/src/worker.c:42");
    }

    #[test]
    fn entry_source_supports_older_dumps_and_partial_debug_information() {
        let mut thread = sample().threads.remove(0);
        assert_eq!(thread.entry_source(), "—");
        thread.entry_line = Some(42);
        assert_eq!(thread.entry_source(), "—");
        thread.entry_file = Some("src/main.c".into());
        assert_eq!(thread.entry_source(), "src/main.c:42");
        thread.entry_line = None;
        assert_eq!(thread.entry_source(), "src/main.c");
        thread.entry_line = Some(0);
        assert_eq!(thread.entry_source(), "src/main.c");
    }

    #[test]
    fn entry_description_combines_available_details_without_empty_placeholders() {
        let mut thread = sample().threads.remove(0);
        assert_eq!(thread.entry_description(), "—");
        thread.entry_point = Some(0x1ba71);
        assert_eq!(thread.entry_description(), "0x0001ba71");
        thread.entry_symbol = Some("bg_thread_main".into());
        assert_eq!(thread.entry_description(), "bg_thread_main (0x0001ba71)");
        thread.entry_file = Some("zephyr/kernel/init.c".into());
        thread.entry_line = Some(283);
        assert_eq!(
            thread.entry_description(),
            "bg_thread_main @ zephyr/kernel/init.c:283 (0x0001ba71)"
        );
        thread.entry_symbol = None;
        assert_eq!(
            thread.entry_description(),
            "zephyr/kernel/init.c:283 (0x0001ba71)"
        );
    }

    #[test]
    fn entry_paths_match_the_tui_without_discarding_the_original_path() {
        let mut thread = sample().threads.remove(0);
        thread.entry_symbol = Some("bg_thread_main".into());
        thread.entry_point = Some(0x1ba71);
        thread.entry_line = Some(283);
        for path in [
            "/Users/developer/workspace/zephyr/kernel/init.c",
            r"C:\workspace\zephyr\kernel\init.c",
            "zephyr/kernel/init.c",
        ] {
            thread.entry_file = Some(path.into());
            assert_eq!(thread.entry_source(), "zephyr/kernel/init.c:283");
            assert_eq!(
                thread.entry_description(),
                "bg_thread_main @ zephyr/kernel/init.c:283 (0x0001ba71)"
            );
            assert_eq!(thread.entry_file.as_deref(), Some(path));
        }
    }

    #[test]
    fn detail_lookup_distinguishes_duplicate_names_and_known_address_reuse() {
        let mut frame = sample();
        let key = frame.threads[0].key();
        let mut duplicate = frame.threads[0].clone();
        duplicate.address += 1;
        frame.threads.insert(0, duplicate);
        assert_eq!(key.in_frame(&frame).unwrap().address, key.address);
        frame.threads[1].entry_point = Some(0x1234);
        assert!(key.in_frame(&frame).is_none());
    }

    #[test]
    fn detail_history_is_per_thread_and_stops_at_selected_time() {
        let mut frames = timed_frames();
        let key = frames[0].threads[0].key();
        for (i, frame) in frames.iter_mut().enumerate() {
            frame.threads[0].runtime.as_mut().unwrap().cpu_normalized = Some(i as f64 * 10.);
        }
        let series = chart_series(&frames, 2, |f| {
            key.in_frame(f)?.runtime.as_ref()?.cpu_normalized
        });
        assert_eq!(series.points.last().unwrap().value, Some(20.));
        assert!(
            series
                .points
                .iter()
                .all(|p| p.value.is_some_and(|v| v <= 20.))
        );
        frames[2].threads.remove(0);
        assert!(key.in_frame(&frames[2]).is_none());
        let missing = chart_series(&frames, 2, |f| {
            key.in_frame(f)?.runtime.as_ref()?.cpu_normalized
        });
        assert_eq!(missing.points.last().unwrap().value, None);
        assert_eq!(missing.segments().count(), 1);
        let resumed = chart_series(&frames, 3, |f| {
            key.in_frame(f)?.runtime.as_ref()?.cpu_normalized
        });
        assert_eq!(
            resumed.points.last(),
            Some(&ChartPoint {
                x: 10.,
                value: Some(30.)
            })
        );
        assert_eq!(resumed.segments().count(), 2);
    }
}

//! Isolated Python decoding: reuse ZView's sequential replay, never a second decoder.
use crate::model::{self, Frame};
use std::{
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

const LIMIT: u64 = 16 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(120);

pub fn load(recording: &Path, elf: &Path, cancel: Arc<AtomicBool>) -> Result<Vec<Frame>, String> {
    let python = std::env::var_os("ZVIEW_PYTHON").unwrap_or_else(|| "python3".into());
    let mut command = Command::new(python);
    command
        .arg("-c")
        .arg(include_str!("../export_recording.py"))
        .arg("--desktop-import")
        .arg(recording)
        .arg(elf);
    // ZView is installed in the selected interpreter. Never infer a sibling
    // checkout or inject this Rust repository's src/ into Python's search path.
    run(command, cancel, TIMEOUT).and_then(|bytes| {
        let text = String::from_utf8(bytes).map_err(|e| format!("Invalid decoder output: {e}"))?;
        model::parse(&text)
    })
}

fn run(
    mut command: Command,
    cancel: Arc<AtomicBool>,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Cannot start recording decoder: {e}. Set ZVIEW_PYTHON to a Python interpreter with ZView dependencies installed."))?;
    let stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let (sender, receiver) = mpsc::channel();
    let output = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout.take(LIMIT + 1).read_to_end(&mut bytes);
        let _ = sender.send(bytes.len() as u64 > LIMIT);
        result.map(|_| bytes)
    });
    let diagnostics = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        // Drain all diagnostics to prevent pipe deadlock, retaining only 8 KiB.
        let mut buffer = [0; 4096];
        while let Ok(count) = stderr.read(&mut buffer) {
            if count == 0 {
                break;
            }
            let keep = count.min(8192_usize.saturating_sub(bytes.len()));
            let _ = bytes.write_all(&buffer[..keep]);
        }
        bytes
    });
    let start = Instant::now();
    let status = loop {
        let failure = if cancel.load(Ordering::Relaxed) {
            Some("Recording import cancelled.")
        } else if start.elapsed() >= timeout {
            Some("Recording import exceeded the 120-second timeout.")
        } else if receiver.try_recv() == Ok(true) {
            Some("Decoded recording exceeds the 16 MiB document limit.")
        } else {
            None
        };
        if let Some(error) = failure {
            let _ = child.kill();
            let _ = child.wait();
            break Err(error.to_string());
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(format!("Cannot wait for recording decoder: {error}"));
            }
        }
    };
    let bytes = output.join().map_err(|_| "Decoder output reader failed.")?;
    let diagnostics = diagnostics.join().unwrap_or_default();
    let status = status?;
    let bytes = bytes.map_err(|e| format!("Cannot read decoder output: {e}"))?;
    if bytes.len() as u64 > LIMIT {
        return Err("Decoded recording exceeds the 16 MiB document limit.".into());
    }
    if !status.success() {
        return Err(format!(
            "Recording decoder failed. Check the recording, matching ELF, and Python dependencies (ZVIEW_PYTHON).\n{}",
            String::from_utf8_lossy(&diagnostics).trim()
        ));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "Requires Python with ZView dependencies; set ZVIEW_PYTHON"]
    fn real_recording_import_preserves_frames_maps_and_reports_bad_inputs() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let fixture = root.join("test-data/zview-bench-60s");
        let frames = load(
            &fixture.join("capture.ndjson.gz"),
            &fixture.join("zephyr.elf"),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        let expected = model::load(&fixture.join("session.json")).unwrap();
        // The preserved JSON was duration-limited to 60 seconds; direct import
        // also retains the last captured frame just beyond that cutoff.
        assert_eq!(frames.len(), expected.len() + 1);
        assert!(frames.last().unwrap().elapsed_seconds.unwrap() > 60.);
        for (frame, expected) in frames.iter().zip(&expected) {
            assert_eq!(frame.elapsed_seconds, expected.elapsed_seconds);
            assert_eq!(frame.threads.len(), expected.threads.len());
            assert_eq!(
                frame.heaps[0].allocated_bytes,
                expected.heaps[0].allocated_bytes
            );
            assert_eq!(frame.workqs.len(), expected.workqs.len());
        }
        assert!(
            frames
                .iter()
                .any(|f| f.heaps[0].chunks.as_ref().is_some_and(|c| c.len() == 130))
        );
        assert!(
            load(
                &fixture.join("capture.ndjson.gz"),
                &fixture.join("missing.elf"),
                Arc::new(AtomicBool::new(false)),
            )
            .unwrap_err()
            .contains("decoder failed")
        );
        assert!(
            load(
                &fixture.join("session.json"),
                &fixture.join("zephyr.elf"),
                Arc::new(AtomicBool::new(false)),
            )
            .unwrap_err()
            .contains("decoder failed")
        );
    }

    #[test]
    fn bridge_handles_success_failure_limits_timeout_and_cancellation() {
        fn python(code: &str) -> Command {
            let mut command = Command::new("python3");
            command.args(["-c", code]);
            command
        }
        let flag = || Arc::new(AtomicBool::new(false));
        assert_eq!(
            run(python("print('[]')"), flag(), TIMEOUT).unwrap(),
            b"[]\n"
        );
        assert!(
            run(
                python("import sys; sys.stderr.write('bad ELF'); sys.exit(1)"),
                flag(),
                TIMEOUT
            )
            .unwrap_err()
            .contains("bad ELF")
        );
        assert!(
            run(python("print('x' * (17 * 1024 * 1024))"), flag(), TIMEOUT)
                .unwrap_err()
                .contains("16 MiB")
        );
        assert!(
            run(
                python("import time; time.sleep(30)"),
                flag(),
                Duration::from_millis(50)
            )
            .unwrap_err()
            .contains("timeout")
        );
        assert!(
            run(
                python("import time; time.sleep(30)"),
                Arc::new(AtomicBool::new(true)),
                TIMEOUT
            )
            .unwrap_err()
            .contains("cancelled")
        );
    }
}
